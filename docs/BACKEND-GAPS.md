# Backend gaps — design → current impl

Tracks Tauri / pier-core capabilities **shown in the `pier-x-copy` design** that
are not yet wired to real commands. Frontend visuals are being ported first;
this file captures everything the UI currently shows as mock / stub / hidden
so the backend work can follow without hunting through git history.

Statuses:

- **stub** — frontend widget is rendered but uses placeholder / empty state
- **hidden** — frontend widget is not rendered yet (no meaningful way to show it without data)
- **partial** — some data is real but the shown fields are only a subset

## MySQL panel

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Splash | "Probe via {ssh target}" activity line with Re-probe button | partial | `dbDetect` already exists; button wires to `refreshDetection` |
| Splash | Instance row meta: `engine`, `addr`, `via`, `user`, `authFrom`, `lastUsed`, `dbs`, `size` | partial | Saved creds expose `{host,port,user,database}`; `engine`, `authFrom`, `lastUsed`, `dbs`, `size` not stored — add to `DbCredential`/detection |
| Splash | `prod / stage / dev / local` env tag per instance | stub | Needs a `env` / `tag` field on saved credentials (user-editable) |
| Header | Stats chips: `{dbs} dbs`, `{size}`, `{ms} roundtrip` | partial | `dbs` = `state.databases.length`; `size` and `ms` roundtrip not measured |
| Schema tree | Views, Functions under a schema | hidden | `mysqlBrowse` only returns tables — extend to list views / routines |
| Schema tree | Row count per table | hidden | Tables are returned as names only — add `information_schema.tables.table_rows` lookup |
| Data tab | Column width resize grip | hidden | Pure frontend (when we add per-column width state) |
| Data tab | Per-column filter row | stub | Frontend-only filter against already-loaded preview rows |
| Data tab | Sort indicator on header | stub | Same — runs against loaded rows, not a server sort |
| Data tab | Inline CRUD (edit / insert / delete with pending commit batch) | hidden | `mysqlExecute` can run DML but there's no "row-level diff → batched UPDATE" command |
| Data tab | Server-side paging (page N of M) | hidden | `DataPreview` is a single capped snapshot — add cursor / limit-offset paging |
| Data tab | Elapsed `ms` on grid toolbar | hidden | Only `queryResult.elapsedMs` exists, not a per-browse number |
| SQL editor | Multiple query tabs | stub | Pure frontend state (add later, no backend) |
| SQL editor | History drawer (recent queries + status) | hidden | Needs a persistent history store (pier-core or localStorage) |
| SQL editor | Favorites | hidden | Needs a saved-query store |
| SQL editor | Format SQL button | hidden | Needs an SQL formatter (pure frontend) |
| SQL editor | EXPLAIN button | hidden | Could call `mysqlExecute('EXPLAIN …')` — wire once editor lands |
| Row detail | Foreign-key "X (N) →" links | hidden | Needs FK introspection via `information_schema.key_column_usage` |
| Structure tab | Columns / Indexes / Foreign keys tables | stub | Only `columns` is returned today — add `indexes` + `foreign keys` |
| Schema tab | Per-table engine / rows / data / idx / updated | hidden | Same `information_schema.tables` lookup as row count |

## PostgreSQL panel

Mirrors MySQL, with the following PG-specific gaps on top:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Schema tree | Schemas under a database (left-rail `public` / `reporting` / …) | hidden | `postgresBrowse` returns only `schemaName` + `tables` for the active schema — add `SELECT schema_name FROM information_schema.schemata` enumeration |
| Schema tree | Views and routines (functions, procedures) | hidden | Extend `postgresBrowse` to return views + routines per schema |
| Header stats | Connection pool / backend count | hidden | `pg_stat_activity` lookup — new command |
| Row detail | `pg_catalog` type decoration (e.g. `shipment_status[]`) | partial | Column types today come through as raw strings; acceptable for MVP |
| Structure tab | Indexes / constraints / foreign keys | hidden | `pg_indexes` + `pg_constraint` lookups |
| Result grid | Array-type / JSONB pretty printing | hidden | Pure frontend — formatter against the raw string from preview |

## Redis panel

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Key list | Per-key type (`STR` / `HASH` / `LIST` / `ZSET` / `STREAM`) badge | hidden | `redisBrowse` only returns the flat key strings — add a lightweight `TYPE` probe in the scan batch (or a parallel MGET of TYPEs, capped) |
| Key list | TTL + size per row | hidden | Same — extend the scan response with `pttl` + approximate size |
| Key tree | Colon-separated hierarchical tree view | stub | Pure frontend once the type/TTL data above lands |
| Key detail | Inline edit (SET / HSET / LPUSH / XADD / ZADD) | hidden | Add a write-side `redisWrite` command family — current `redisExecute` runs raw strings but has no structured edit |
| Key detail | Rename / Delete actions | hidden | Need `RENAME` + `DEL` through the existing `redisExecute`, but we'd want a confirm-guarded command |
| Header stats | Round-trip `ms` chip | hidden | `redisBrowse` doesn't measure — add RTT to the response |
| Scan | Cursor-based paging (load-more) | hidden | `SCAN` cursor isn't threaded through the existing command — add cursor state + `scanCursor` in / out |
| Scan | Pattern + DB change without running the full browse | partial | `redisBrowse` already does this — UI just needs to feed the new values |
| CLI | Rich REPL (history, up-arrow recall) | stub | Current impl runs a single statement; history belongs in frontend state |

## SQLite panel

The SQLite panel already has more backend coverage than the design — it
adds a remote capability probe and scan-directory flow the design doesn't
have. Remaining visual / data gaps:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Splash — saved profiles | Persisted SQLite file paths as reusable profiles | hidden | `DbCredential` already supports `kind: "sqlite"` + `sqlitePath` but nothing persists a saved SQLite target yet; wire `DbAddCredentialDialog` for SQLite |
| Splash — env tag | `local` vs `prod · remote read` tag | stub | Frontend-only once saved profiles land |
| Structure tab | Indexes + triggers per table | hidden | `PRAGMA index_list` / `trigger_list` lookups in both `sqliteBrowse` variants |
| Connected header | Rough "{size}" stat for the opened file | hidden | `sqliteBrowse` could return `st.st_size`; `sqliteBrowseRemote` already has the candidate size from `sqliteFindInDir` |
| Query editor | Multi-statement scripts + per-statement timing | partial | `sqliteExecute` runs one statement at a time — extend for semicolon-split runs |

## SFTP file editor dialog

Visual chrome is now design-matched (header chips + View/Edit segment +
toolbar + footer). Functional gaps from the design:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Header chips | `owner` chip (e.g. `deploy:deploy`) | hidden | `sftpReadText` returns size/permissions/modified; owner/group needs an extra `sftp_stat` lookup |
| Toolbar | Dedicated Find vs Find-and-Replace buttons | partial | The design opens an in-dialog bar; we defer to CodeMirror6's built-in panel via the same `Search` icon — acceptable but slightly different look |
| Toolbar | Download to local disk | stub | Implemented frontend-only (in-memory blob → <a download>) — sandbox environments may block it |
| Toolbar | Copy path | stub | Frontend-only via clipboard — done |
| Footer | EOL detection (LF / CRLF) | hidden | Backend returns raw content; add a tiny detector or return it from `sftpReadText` |
| Footer | Encoding detection beyond UTF-8 vs lossy | hidden | Currently hardcoded to "UTF-8"; extend `sftpReadText` with a detected charset |
| View mode | Read-only state is enforced by disabling the CM6 editor | partial | The segment toggle reconfigures `EditorState.readOnly` — good for the visual, but no "pretty print / render markdown" differentiation exists yet |

## Log viewer

The current **`LogViewerPanel`** now matches the design's main surface
(level chips + counts, search, wrap/clear/download, clickable-line → detail
pane). The design's **dialog** form factor with a wide left rail
(Sources / Time Range / Columns / Context) doesn't fit the right-panel
layout and is out of scope for the port.

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Left rail — multiple sources list | Tile per detected log file (size / rate per source) | hidden | Requires multi-stream support in `logStreamStart` + `logStreamDrain`, and a per-host "recent log files" discovery pass |
| Left rail — time range chips (1m / 15m / 1h / 24h / all) | hidden | The current stream is a live tail — time-range filter would need a back-fill read (`journalctl --since`, `tail -n` etc.) |
| Left rail — column visibility toggle | stub | Frontend-only — line # / timestamp / level / source columns all render today; add a popover to hide individually |
| Left rail — context-lines picker (off / ±1 / ±3 / ±5) | hidden | Pure frontend around matches; only makes sense once search-hit navigation is wired |
| Line detail pane | KV grid (timestamp / level / source / message / host) | partial | Implemented as a slide-up pane in the panel body; design's full dialog form-factor is deferred |
| Streaming rate chip ("42 l/s") | hidden | Add an EMA counter in the drain loop and surface it on the panel header |
| Search — prev/next hit navigation + hit count | hidden | The current `searchText` filters the list but doesn't number hits — add a hit index + scroll-into-view |
| Dialog form factor | Full-screen modal with left rail + main + detail | hidden | Out of scope: the right-panel docking is the canonical home for logs; revisit only if we grow a detachable log window |

## Docker panel — Compose

The current Docker panel has a **Projects** tab that label-groups running
containers by `com.docker.compose.project`. The design's **Compose** tab
is YAML-file-oriented (picks a `docker-compose.*.yml`, shows service
replica counts, and exposes file-level actions). Bridging requires:

| Area | Status | Needed Tauri command(s) / notes |
|---|---|---|
| Pick a `docker-compose.*.yml` on the remote host | hidden | `sftpBrowse` exists; add a dedicated "discover compose files under cwd + common locations" scan (similar to sqliteFindInDir) |
| Parse compose file → services/replicas/image | hidden | New pier-core parser (serde_yaml) — expose `dockerComposeInspect(path)` |
| `docker compose up -d` / `down` / `restart` / `pull` / `build` from the panel | hidden | Shell out to `docker compose -f <path> <action>`; stream stdout/stderr to the existing log pipeline |
| Per-service `logs` / `restart` / `stop` actions in the services table | partial | Existing container commands cover this once we can map `service → container id` |
| Health / replica summary (e.g. "4/5 healthy") | partial | `docker compose ps --format json` gives it; current panel counts `running/total` only |
| "Active compose file" as tab state (remembered per SSH host) | hidden | Persist in `tab` state + connection profile |
