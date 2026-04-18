use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use std::error::Error;
use std::fmt;

use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

static LOGGER: OnceLock<FileLogger> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init_logging() -> Result<PathBuf, LoggingInitError> {
    if let Some(path) = LOG_PATH.get() {
        return Ok(path.clone());
    }

    let path = default_log_path().ok_or(LoggingInitError::NoLogDir)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    rotate_existing_log(&path)?;

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;
    let level = level_from_env();
    let logger = FileLogger {
        level,
        file: Mutex::new(file),
    };

    LOGGER
        .set(logger)
        .map_err(|_| LoggingInitError::AlreadyInitialized)?;
    LOG_PATH
        .set(path.clone())
        .map_err(|_| LoggingInitError::AlreadyInitialized)?;

    let logger = LOGGER.get().ok_or(LoggingInitError::AlreadyInitialized)?;
    install_logger(logger, level)?;
    install_panic_hook();

    log::info!(
        "diagnostics initialized level={} file={}",
        level.as_str(),
        path.display()
    );

    Ok(path)
}

pub fn current_log_path() -> Option<&'static Path> {
    LOG_PATH.get().map(PathBuf::as_path)
}

#[derive(Debug)]
pub enum LoggingInitError {
    NoLogDir,
    Io(io::Error),
    AlreadyInitialized,
    SetLogger(SetLoggerError),
}

impl fmt::Display for LoggingInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLogDir => write!(f, "no usable log directory"),
            Self::Io(err) => write!(f, "failed to initialize file logger: {err}"),
            Self::AlreadyInitialized => write!(f, "logger already initialized"),
            Self::SetLogger(err) => write!(f, "failed to register logger: {err}"),
        }
    }
}

impl Error for LoggingInitError {}

impl From<io::Error> for LoggingInitError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<SetLoggerError> for LoggingInitError {
    fn from(value: SetLoggerError) -> Self {
        Self::SetLogger(value)
    }
}

struct FileLogger {
    level: LevelFilter,
    file: Mutex<File>,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("main");
        let line = format!(
            "{}.{:03} {:<5} [{}] {}: {}\n",
            now.as_secs(),
            now.subsec_millis(),
            record.level().as_str(),
            thread_name,
            record.target(),
            record.args()
        );

        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }

        if matches!(record.level(), Level::Warn | Level::Error) {
            eprint!("{line}");
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

fn level_from_env() -> LevelFilter {
    let raw = env::var("PIER_LOG")
        .ok()
        .or_else(|| env::var("RUST_LOG").ok())
        .unwrap_or_else(|| "info".to_string());

    match raw.trim().to_ascii_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "warn" | "warning" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        "off" => LevelFilter::Off,
        _ => LevelFilter::Info,
    }
}

fn default_log_path() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("PIER_LOG_FILE") {
        let explicit = explicit.trim();
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }

    workspace_root().map(|root| root.join("pier-ui-gpui.log"))
}

fn workspace_root() -> Option<PathBuf> {
    let cwd = env::current_dir().ok();
    if let Some(ref dir) = cwd {
        if looks_like_workspace_root(dir) {
            return Some(dir.clone());
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(parent) = manifest_dir.parent() {
        let parent = parent.to_path_buf();
        if looks_like_workspace_root(&parent) {
            return Some(parent);
        }
    }

    cwd
}

fn looks_like_workspace_root(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file() && dir.join("pier-ui-gpui").is_dir()
}

fn rotate_existing_log(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let previous = previous_log_path(path);
    if previous.exists() {
        fs::remove_file(&previous)?;
    }
    fs::rename(path, previous)
}

fn previous_log_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("pier-ui-gpui");
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("log");
    let file_name = format!("{stem}.previous.{ext}");
    path.with_file_name(file_name)
}

fn install_logger(logger: &'static FileLogger, level: LevelFilter) -> Result<(), SetLoggerError> {
    log::set_logger(logger)?;
    log::set_max_level(level);
    Ok(())
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        log::error!("panic: {panic_info}");
        default_hook(panic_info);
    }));
}
