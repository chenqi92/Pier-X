//! File-preview helpers — UI-agnostic decoding/parsing of remote
//! files for Pier-X's SFTP multi-format viewer.
//!
//! Everything here works on plain bytes (or a live [`SftpClient`])
//! and returns plain Rust types, so the Tauri layer can expose thin
//! commands without any rendering logic leaking down here.
//!
//! ## Pieces
//!
//!   * [`detect_prefix`] — cheap text-vs-binary + charset guess on a
//!     small prefix, so the viewer can pick text/hex from the first
//!     window instead of after a whole-file read.
//!   * [`stream_remote_text`] — windowed, incrementally-decoded text
//!     streaming over [`SftpClient::read_range`], for instant-open
//!     large-file / log viewing without buffering the whole file.
//!   * [`parse_spreadsheet`] / [`parse_csv`] — normalize xlsx / xls /
//!     ods / csv into a simple column+row table the frontend renders
//!     in its shared preview table.
//!   * [`decode_image_to_png`] — re-encode formats the WebView can't
//!     decode natively (TIFF, …) to PNG bytes.

use std::io::Cursor;

use tokio_util::sync::CancellationToken;

use crate::ssh::error::{Result, SshError};
use crate::ssh::sftp::SftpClient;

// ── Text / binary detection ────────────────────────────────────

/// Result of sniffing a file's leading bytes.
#[derive(Clone, Debug)]
pub struct PrefixDetection {
    /// True when the prefix looks like decodable text, false when it
    /// looks like binary (NUL bytes / control noise).
    pub is_text: bool,
    /// The encoding to decode the file with when `is_text` is true.
    /// Defaults to UTF-8 for binary content (unused in that case).
    pub encoding: &'static encoding_rs::Encoding,
    /// Human-readable encoding label (e.g. `UTF-8`, `GBK`,
    /// `Shift_JIS`, `UTF-16LE`, or `binary`).
    pub label: String,
}

/// Sniff up to a few KiB of a file's leading bytes to decide whether
/// it is text and, if so, which charset to decode it with.
///
/// Order matters: a BOM is checked first (it pins UTF-8/16/32, which
/// the statistical detector can't recognise for UTF-16 without one),
/// then [`content_inspector`] rules out binary, then [`chardetng`]
/// (Firefox's detector) guesses the legacy charset for everything
/// else. Pass the first 8–64 KiB you already fetched for the first
/// window — there's no need to read the whole file.
pub fn detect_prefix(prefix: &[u8]) -> PrefixDetection {
    use content_inspector::{inspect, ContentType};

    let ct = inspect(prefix);
    let bom = match ct {
        ContentType::UTF_8_BOM => Some((encoding_rs::UTF_8, "UTF-8-BOM")),
        ContentType::UTF_16LE => Some((encoding_rs::UTF_16LE, "UTF-16LE")),
        ContentType::UTF_16BE => Some((encoding_rs::UTF_16BE, "UTF-16BE")),
        _ => None,
    };
    if let Some((encoding, label)) = bom {
        return PrefixDetection {
            is_text: true,
            encoding,
            label: label.to_string(),
        };
    }
    if !ct.is_text() {
        return PrefixDetection {
            is_text: false,
            encoding: encoding_rs::UTF_8,
            label: "binary".to_string(),
        };
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(prefix, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);
    PrefixDetection {
        is_text: true,
        encoding,
        label: encoding.name().to_string(),
    }
}

// ── Streaming text ─────────────────────────────────────────────

/// One decoded slice of a streamed text file, handed to the
/// per-chunk callback in [`stream_remote_text`].
#[derive(Clone, Debug)]
pub struct TextStreamChunk {
    /// `"text"` for decoded content, `"binary"` for the single
    /// "this file is not text, switch to hex" signal.
    pub kind: String,
    /// Encoding label the content was decoded with.
    pub encoding: String,
    /// Decoded UTF-8 text for this slice (empty for the binary
    /// signal and for the final EOF marker).
    pub text: String,
    /// Byte offset immediately after this slice — feed it back as
    /// `start` to continue ("load more") past a truncation cap.
    pub next_offset: u64,
    /// Total file size in bytes, for the viewer's progress math.
    pub total_size: u64,
    /// True on the terminal message (EOF reached, or binary signal).
    pub done: bool,
    /// True when streaming stopped at `max_bytes` before EOF.
    pub truncated: bool,
}

/// Stream a remote file as decoded text in windows, invoking
/// `on_chunk` for each slice as it arrives.
///
/// Reads `chunk_bytes` at a time from `start` via
/// [`SftpClient::read_range`], so the first window — and therefore
/// the first screen of text — lands after a single round trip
/// regardless of file size. The charset is sniffed from the first
/// window; if it looks binary, a single `kind = "binary"` chunk is
/// emitted and the function returns (the caller should fall back to
/// the hex view). Decoding is incremental ([`encoding_rs::Decoder`]
/// carries partial multibyte sequences across window boundaries), so
/// chunk splits never corrupt characters.
///
/// Stops at `max_bytes` (emitting a `truncated` terminal chunk so the
/// UI can offer "load more" from `next_offset`) or at EOF (emitting a
/// `done` terminal chunk). Cancellation is checked between windows.
#[allow(clippy::too_many_arguments)]
pub async fn stream_remote_text<F>(
    client: &SftpClient,
    path: &str,
    start: u64,
    max_bytes: u64,
    chunk_bytes: usize,
    total_size: u64,
    mut on_chunk: F,
    cancel: Option<&CancellationToken>,
) -> Result<()>
where
    F: FnMut(TextStreamChunk) + Send,
{
    let mut offset = start;
    let mut produced: u64 = 0;
    let mut decoder: Option<encoding_rs::Decoder> = None;
    let mut label = String::new();

    loop {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                return Ok(());
            }
        }
        let remaining = max_bytes.saturating_sub(produced);
        if remaining == 0 {
            on_chunk(TextStreamChunk {
                kind: "text".to_string(),
                encoding: label.clone(),
                text: String::new(),
                next_offset: offset,
                total_size,
                done: true,
                truncated: true,
            });
            return Ok(());
        }
        let want = chunk_bytes.min(remaining as usize);
        let bytes = client.read_range(path, offset, want).await?;
        let n = bytes.len();

        if n == 0 {
            // EOF — flush any buffered partial sequence, then mark done.
            if let Some(dec) = decoder.as_mut() {
                let mut tail = String::new();
                let _ = dec.decode_to_string(&[], &mut tail, true);
                if !tail.is_empty() {
                    on_chunk(TextStreamChunk {
                        kind: "text".to_string(),
                        encoding: label.clone(),
                        text: tail,
                        next_offset: offset,
                        total_size,
                        done: false,
                        truncated: false,
                    });
                }
            }
            on_chunk(TextStreamChunk {
                kind: "text".to_string(),
                encoding: label.clone(),
                text: String::new(),
                next_offset: offset,
                total_size,
                done: true,
                truncated: false,
            });
            return Ok(());
        }

        if decoder.is_none() {
            let det = detect_prefix(&bytes[..n.min(8192)]);
            if !det.is_text {
                on_chunk(TextStreamChunk {
                    kind: "binary".to_string(),
                    encoding: det.label,
                    text: String::new(),
                    next_offset: offset,
                    total_size,
                    done: true,
                    truncated: false,
                });
                return Ok(());
            }
            label = det.label;
            decoder = Some(det.encoding.new_decoder_with_bom_removal());
        }

        let dec = decoder.as_mut().expect("decoder set above");
        let mut text = String::with_capacity(n + n / 2);
        let _ = dec.decode_to_string(&bytes, &mut text, false);
        offset += n as u64;
        produced += n as u64;
        on_chunk(TextStreamChunk {
            kind: "text".to_string(),
            encoding: label.clone(),
            text,
            next_offset: offset,
            total_size,
            done: false,
            truncated: false,
        });
    }
}

/// Blocking wrapper for [`stream_remote_text`], driving it on the
/// shared runtime so a synchronous Tauri command body can forward
/// each chunk to an IPC channel.
#[allow(clippy::too_many_arguments)]
pub fn stream_remote_text_blocking<F>(
    client: &SftpClient,
    path: &str,
    start: u64,
    max_bytes: u64,
    chunk_bytes: usize,
    total_size: u64,
    on_chunk: F,
    cancel: Option<&CancellationToken>,
) -> Result<()>
where
    F: FnMut(TextStreamChunk) + Send,
{
    crate::ssh::runtime::shared().block_on(stream_remote_text(
        client,
        path,
        start,
        max_bytes,
        chunk_bytes,
        total_size,
        on_chunk,
        cancel,
    ))
}

// ── Tabular formats (spreadsheet / CSV) ────────────────────────

/// A normalized table extracted from a spreadsheet or CSV file.
#[derive(Clone, Debug, Default)]
pub struct TablePreview {
    /// Names of every sheet in the workbook (single-element
    /// `["csv"]` for CSV). Lets the frontend offer a sheet picker.
    pub sheet_names: Vec<String>,
    /// Index of the sheet this preview was extracted from.
    pub sheet_index: usize,
    /// Header row — the first row of the sheet/file.
    pub columns: Vec<String>,
    /// Body rows, each padded/truncated to `columns.len()`.
    pub rows: Vec<Vec<String>>,
    /// True when `rows` was capped at the requested row limit.
    pub truncated: bool,
}

fn cell_to_string(cell: &calamine::Data) -> String {
    if matches!(cell, calamine::Data::Empty) {
        String::new()
    } else {
        cell.to_string()
    }
}

/// Parse a spreadsheet (`xlsx` / `xlsb` / `xls` / `ods`) from bytes
/// into a [`TablePreview`].
///
/// The first row of the selected sheet becomes the header; the rest
/// become body rows, capped at `max_rows` (with `truncated` set when
/// the cap bites). `sheet_index` selects which sheet to read.
pub fn parse_spreadsheet(
    bytes: Vec<u8>,
    sheet_index: usize,
    max_rows: usize,
) -> Result<TablePreview> {
    use calamine::{open_workbook_auto_from_rs, Reader};

    let cursor = Cursor::new(bytes);
    let mut workbook = open_workbook_auto_from_rs(cursor)
        .map_err(|e| SshError::InvalidConfig(format!("spreadsheet open failed: {e}")))?;
    let sheet_names = workbook.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Ok(TablePreview::default());
    }
    let idx = sheet_index.min(sheet_names.len() - 1);
    let name = sheet_names[idx].clone();
    let range = workbook
        .worksheet_range(&name)
        .map_err(|e| SshError::InvalidConfig(format!("read sheet '{name}' failed: {e}")))?;

    let mut iter = range.rows();
    let columns: Vec<String> = iter
        .next()
        .map(|r| r.iter().map(cell_to_string).collect())
        .unwrap_or_default();
    let width = columns.len().max(1);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    for row in iter {
        if rows.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut cells: Vec<String> = row.iter().map(cell_to_string).collect();
        cells.resize(width, String::new());
        rows.push(cells);
    }

    Ok(TablePreview {
        sheet_names,
        sheet_index: idx,
        columns,
        rows,
        truncated,
    })
}

/// Parse delimited text (CSV / TSV) from bytes into a
/// [`TablePreview`]. The delimiter is `\t` when `tab` is true, else
/// `,`. The first record is treated as the header row.
pub fn parse_csv(bytes: Vec<u8>, tab: bool, max_rows: usize) -> Result<TablePreview> {
    let delimiter = if tab { b'\t' } else { b',' };
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(Cursor::new(bytes));

    let mut records = reader.records();
    let columns: Vec<String> = match records.next() {
        Some(Ok(rec)) => rec.iter().map(|s| s.to_string()).collect(),
        Some(Err(e)) => return Err(SshError::InvalidConfig(format!("csv parse failed: {e}"))),
        None => Vec::new(),
    };
    let width = columns.len().max(1);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    for rec in records {
        if rows.len() >= max_rows {
            truncated = true;
            break;
        }
        let rec = rec.map_err(|e| SshError::InvalidConfig(format!("csv parse failed: {e}")))?;
        let mut cells: Vec<String> = rec.iter().map(|s| s.to_string()).collect();
        cells.resize(width, String::new());
        rows.push(cells);
    }

    Ok(TablePreview {
        sheet_names: vec!["csv".to_string()],
        sheet_index: 0,
        columns,
        rows,
        truncated,
    })
}

// ── Images the WebView can't decode natively ───────────────────

/// Decode an image the WebView can't render natively (TIFF, …) and
/// re-encode it as PNG bytes that a plain `<img>` can display.
pub fn decode_image_to_png(bytes: &[u8]) -> Result<Vec<u8>> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| SshError::InvalidConfig(format!("image decode failed: {e}")))?;
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| SshError::InvalidConfig(format!("png encode failed: {e}")))?;
    Ok(out.into_inner())
}
