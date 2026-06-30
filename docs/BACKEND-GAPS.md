# Backend gaps — design → current impl

Tracks Tauri / pier-core capabilities **shown in the `pier-x-copy` design** that
are not yet wired to real commands. Frontend visuals are being ported first;
this file captures everything the UI currently shows as mock / stub / hidden
so the backend work can follow without hunting through git history.

Statuses:

- **stub** — frontend widget is rendered but uses placeholder / empty state
- **hidden** — frontend widget is not rendered yet (no meaningful way to show it without data)
- **partial** — some data is real but the shown fields are only a subset
- **shipped** — closed by a merged PR; the row stays here as historical context until the next doc sweep
- **blocked-by-spec** — closing the gap requires changes to PRODUCT-SPEC.md first; flagged so a future planner doesn't quietly re-implement the design without revisiting the spec conversation

## Recently closed (PRs in flight or merged)

Tracking what's landed since the original gap pass — listed here for skim
convenience; the per-section rows below carry the same status. Update this
header on merge and drop rows once they're confirmed shipped.

| PR branch | Closes |
|---|---|
| `feat/redis-key-meta` | Redis: per-key kind/TTL chips, cursor paging, RTT chip |
| `feat/sqlite-cluster` | SQLite: indexes/triggers (Structure tab), file-size chip, multi-statement scripts |
| `feat/sftp-cluster` | SFTP: owner/group column + chip, EOL detection, encoding detection |
| `feat/log-viewer-cluster` | Log viewer: streaming rate chip, time-range filter (client-side over the live ring) |
| `feat/pg-schema-picker-pool` | PG: schema picker (left rail), `pg_stat_activity` connection-pool chip |
| `feat/mysql-paging-history` | MySQL: server-side paging (`offset` / `limit` / `total_rows`), localStorage history persistence (200 cap, per-engine bucket) |
| `feat/docker-compose-derived` | Docker Compose: per-service replica chip, service-level Restart-all / Stop-all (no compose CLI — pure label-derived per spec §5.4) |
| `feat/db-structure-keys` | MySQL/PG: indexes + foreign keys in Structure tab |
| `feat/mysql-schema-enrichment`, `feat/pg-schema-enrichment` | Views/routines + table-meta tooltip in schema tree |
| `feat/sql-explain-format` | EXPLAIN + Format SQL buttons across MySQL/PG/SQLite |
| `feat/result-grid-json-pretty` | Result grid: JSONB / array pretty-print on hover |
| `feat/terminal-history-persistence` | Smart Mode terminal: per-shell history persisted to `terminal-history-<shell>.jsonl` under the app data dir (`pier-core::paths::data_dir()`) |
| `feat/web-server-unify` | Web Server panel consolidates nginx/Apache/Caddy under one `rightTool: "webserver"`. Detection (`web_server_detect`), generic validate/reload (`web_server_validate`/`_reload`), shared layout/read/save pipeline (`web_server_layout`/`_read_file`/`_save_file`), Apache site toggle (`web_server_toggle_site`), new-site wizard (`web_server_create_site`), Caddy parser/renderer (`caddy_parse`/`_render`, 5 tests), Apache parser/renderer (`apache_parse`/`_render`, 7 tests). Apache catalog 9 features, Caddy catalog 9 features. |

## MySQL panel

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Splash | "Probe via {ssh target}" activity line with Re-probe button | partial | `dbDetect` already exists; button wires to `refreshDetection` |
| Splash | Instance row meta: `engine`, `addr`, `via`, `user`, `authFrom`, `lastUsed`, `dbs`, `size` | shipped | `MySqlPanel` renders `engine` / `addr` / `via` / `user` / `authHint` / `stats` / `lastUsed` on every saved row via `DbSplashRow` |
| Splash | `prod / stage / dev / local` env tag per instance | shipped | `inferEnv(cred.label)` on the splash row; `DbSplashRow` renders `DbEnvTag` next to the connection label |
| Header | Stats chips: `{dbs} dbs`, `{size}`, `{ms} roundtrip` | partial | `MySqlPanel` header renders database + table counts only; total `size` and `ms` roundtrip chips still absent |
| Schema tree | Views, Functions under a schema | shipped | `feat/mysql-schema-enrichment` — `mysqlBrowse` now returns views + routines |
| Schema tree | Row count per table | shipped | `feat/mysql-schema-enrichment` — table-meta tooltip carries `table_rows` |
| Data tab | Column width resize grip | shipped | `DbResultGrid` handles `startColResize` with per-column width state |
| Data tab | Per-column filter row | shipped | `DbResultGrid` renders an opt-in filter row above the data rows |
| Data tab | Sort indicator on header | shipped | `DbResultGrid` renders sort direction icon on the active column header |
| Data tab | Inline CRUD (edit / insert / delete with pending commit batch) | shipped | `feat/db-grid-crud` — `DbResultGrid` collects pending mutations, `mutationToSql` builds quoted UPDATE/INSERT/DELETE per dialect, single Commit button fans them through `mysqlExecute` (works for both MySQL and PG) |
| Data tab | Server-side paging (page N of M) | shipped | `feat/mysql-paging-history` — `mysql_browse(offset, limit)` + `total_rows`; pager + page-size dropdown in toolbar |
| Data tab | Elapsed `ms` on grid toolbar | shipped | `MySqlPanel` records `state.browseElapsedMs` per `mysqlBrowse` round-trip and renders the chip in the data toolbar next to the pager |
| SQL editor | Multiple query tabs | shipped | `useDbSqlTabs` supports up to 8 concurrent tabs per panel |
| SQL editor | History drawer (recent queries + status) | shipped | `feat/mysql-paging-history` — `useDbSqlTabs` persists per-engine to localStorage (`pier-x:sql-history:<engine>`), 200-entry cap |
| SQL editor | Favorites | shipped | `useDbSqlTabs` persists pinned queries per engine in `pier-x:sql-favorites:<engine>` (50-entry cap); editor exposes Add/Remove/Pick from the rail |
| SQL editor | Format SQL button | shipped | `feat/sql-explain-format` — sql-formatter dep + button on all three SQL panels |
| SQL editor | EXPLAIN button | shipped | `feat/sql-explain-format` — runs `EXPLAIN <sql>` via existing execute |
| SQL editor | EXPLAIN ANALYZE plan tree | shipped | `feat/explain-plan-tree` — `Plan` button runs `EXPLAIN (ANALYZE, FORMAT JSON, BUFFERS)` (PG) or `EXPLAIN FORMAT=JSON` (MySQL); JSON parsed by `lib/explainPlan.ts` into a unified `PlanNode`; rendered hierarchically by `ExplainPlanView` with rows/cost/actual-time/buffers chips |
| Row detail | Foreign-key "X (N) →" links | shipped | `buildFkEdges()` feeds FK metadata to `DbRowDetail`; `lib/fkNav.ts` builds click handlers that route to the target table via `onNavigate(sql)` |
| Structure tab | Columns / Indexes / Foreign keys tables | shipped | `feat/db-structure-keys` — indexes + FK sections under the column grid |
| Schema tab | Per-table engine / rows / data / idx / updated | shipped | `feat/mysql-schema-enrichment` — table-meta tooltip exposes engine / rows / size |

## PostgreSQL panel

Mirrors MySQL, with the following PG-specific gaps on top:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Schema tree | Schemas under a database (left-rail `public` / `reporting` / …) | shipped | `feat/pg-schema-picker-pool` — `postgresBrowse` returns `schemas[]`; tree renders schema picker |
| Schema tree | Views and routines (functions, procedures) | shipped | `feat/pg-schema-enrichment` — views + routines listed per schema |
| Header stats | Connection pool / backend count | shipped | `feat/pg-schema-picker-pool` — `pool_status` walks `pg_stat_activity`; chip shows `{active}/{total}` |
| Row detail | `pg_catalog` type decoration (e.g. `shipment_status[]`) | hidden | `DbRowDetail` renders `column.type` as the raw string; no `[]` array suffix or enum-name lookup yet |
| Structure tab | Indexes / constraints / foreign keys | shipped | `feat/db-structure-keys` — `pg_index` + `pg_constraint` walks |
| Result grid | Array-type / JSONB pretty printing | shipped | `feat/result-grid-json-pretty` — formatter on hover for JSONB / array cells |

## Redis panel

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Key list | Per-key type (`STR` / `HASH` / `LIST` / `ZSET` / `STREAM`) badge | shipped | `feat/redis-key-meta` — TYPE pipeline after SCAN, badge per row |
| Key list | TTL + size per row | shipped | `feat/redis-key-meta` — PTTL pipeline; ∞ / s / m / h / d chip |
| Key tree | Colon-separated hierarchical tree view | shipped | `feat/redis-edits-tree` — `RedisKeyList` collapses on `:` (configurable separator); tree mode is the default |
| Key detail | Inline edit (SET / HSET / LPUSH / XADD / ZADD) | shipped | `feat/redis-edits-tree` — `RedisEdit` op union covers string/hash/list/set/zset + TTL; XADD stream stays read-only by design |
| Key detail | Rename / Delete actions | shipped | `feat/redis-edits-tree` — confirm-guarded `RENAMENX` (safe) + `DEL` Tauri commands |
| Header stats | Round-trip `ms` chip | shipped | `feat/redis-key-meta` — `rtt_ms` measured around the SCAN+TYPE+PTTL pipeline |
| Scan | Cursor-based paging (load-more) | shipped | `feat/redis-key-meta` — `next_cursor` + Load-more button; merged-and-deduped append |
| Scan | Pattern + DB change without running the full browse | hidden | Pattern input still requires explicit Enter / Scan click; DB-index change updates the tab state but doesn't auto-rebrowse — the no-spinner UX is unimplemented |
| CLI | Rich REPL (history, up-arrow recall) | shipped | `RedisPanel` persists CLI history to localStorage (`pier-x:redis-cli-history-v1`, 50-entry cap) and binds ArrowUp / ArrowDown to navigate while preserving the in-progress draft. |

## SQLite panel

The SQLite panel already has more backend coverage than the design — it
adds a remote capability probe and scan-directory flow the design doesn't
have. Remaining visual / data gaps:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Splash — saved profiles | Persisted SQLite file paths as reusable profiles | hidden | Blocked: `dbCredSave` keys to an SSH connection_index; local SQLite has no anchor. Closing this needs a new "global" credential bucket — out of scope for `feat/sqlite-cluster` |
| Splash — env tag | `local` vs `prod · remote read` tag | hidden | `SqlitePanel` hardcodes `env: "unknown"` for every detected row; depends on the saved-profiles bucket above |
| Structure tab | Indexes + triggers per table | shipped | `feat/sqlite-cluster` — `PRAGMA index_list` / `index_info` / `sqlite_master` triggers; rendered under the column grid |
| Connected header | Rough "{size}" stat for the opened file | shipped | `feat/sqlite-cluster` — `std::fs::metadata` locally; `stat -c %s ‖ stat -f %z` over SSH |
| Query editor | Multi-statement scripts + per-statement timing | shipped | `feat/sqlite-cluster` — new `sqlite_execute_script` splits on top-level `;` (quote/comment-aware) and returns per-statement timing |

## SFTP file editor dialog

Visual chrome is now design-matched (header chips + View/Edit segment +
toolbar + footer). Functional gaps from the design:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Header chips | `owner` chip (e.g. `deploy:deploy`) | shipped | `feat/sftp-cluster` — `RemoteFileEntry` carries `owner` / `group` (named, falling back to numeric); rendered as a head chip + browser column |
| Toolbar | Dedicated Find vs Find-and-Replace buttons | shipped | `SftpEditorDialog` exposes the CodeMirror 6 search panel via the toolbar Search icon; design accepted in lieu of an in-dialog bar |
| Toolbar | Download to local disk | shipped | `SftpEditorDialog` calls `sftp_download` directly to write the bytes to a user-chosen path |
| Toolbar | Copy path | stub | Frontend-only via clipboard — done |
| Footer | EOL detection (LF / CRLF) | shipped | `feat/sftp-cluster` — `detect_eol` walks the decoded text once, picks the dominant kind, ties → "mixed" |
| Footer | Encoding detection beyond UTF-8 vs lossy | shipped | `feat/sftp-cluster` — 3-byte BOM + NUL-scan classifier, surfaces utf-8 / utf-8-bom / utf-16-le / utf-16-be / binary |
| View mode | Read-only state is enforced by disabling the CM6 editor | shipped | `SftpEditorDialog` adds a `render` mode that runs `renderMarkdown` for `*.md` files alongside the read-only `view` mode |

## Log viewer

The current **`LogViewerPanel`** now matches the design's main surface
(level chips + counts, search, wrap/clear/download, clickable-line → detail
pane). The design's **dialog** form factor with a wide left rail
(Sources / Time Range / Columns / Context) doesn't fit the right-panel
layout and is out of scope for the port.

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Left rail — multiple sources list | Tile per detected log file (size / rate per source) | shipped | `LogViewerPanel` renders a pinned-rail of alternates plus the file-mode dropdown driven by per-host source discovery |
| Left rail — time range chips (1m / 15m / 1h / 24h / all) | shipped | `runBackfill()` compiles `--since` flags via `lib/logSource.ts` for journalctl and falls back to `tail -n` elsewhere; chips drive both client-side filter and real backfill |
| Left rail — column visibility toggle | shipped | LogViewerDialog already had this — column checkboxes filter the rendered cells |
| Left rail — context-lines picker (off / ±1 / ±3 / ±5) | hidden | Pure frontend around matches; only makes sense once search-hit navigation is wired (currently `disabled` in the dialog) |
| Line detail pane | KV grid (timestamp / level / source / message / host) | partial | Implemented as a slide-up pane in the panel body; design's full dialog form-factor is deferred |
| Streaming rate chip ("42 l/s") | shipped | `feat/log-viewer-cluster` — 30/70 EMA driven from drain cadence; idle decay so quiet streams fall to zero |
| Search — prev/next hit navigation + hit count | shipped | LogViewerDialog already had `nextHit` / `prevHit` / `{n}/{total}` chip / scroll-into-view |
| Dialog form factor | Full-screen modal with left rail + main + detail | hidden | Out of scope: the right-panel docking is the canonical home for logs; revisit only if we grow a detachable log window |

## Web Server panel (nginx / Apache / Caddy)

Detection, raw editing, save→validate→reload, site toggle, new-site
wizard, parsers, and feature catalogs all shipped via
`feat/web-server-unify`. Outstanding items:

| Area | Status | Notes |
|---|---|---|
| Apache structured tree view | shipped | `ApacheTreeView` mirrors `CaddyTreeView` — pencil edit (name + args) / add-top-level / add-child / trash-remove. AST mutations round-trip through `apache_render` to update the dirty buffer. |
| Caddy editable tree mode | shipped | `CaddyTreeView` supports add-top-level / add-child / pencil-edit (name + args) / trash-remove on every node; AST mutations round-trip through `caddy_render` to update the dirty buffer. |
| Apache feature catalog beyond 9 | shipped | `apacheFeatures.ts` exports 16 features — base 9 plus `<IfModule>` / `mod_deflate` / `mod_expires` / `Listen` / `ServerTokens` / `Timeout`-`KeepAlive` / `<RequireAll>`-`<RequireAny>`. |
| Caddy feature catalog beyond 9 | shipped | `caddyFeatures.ts` exports 16 features — base 9 plus `handle_path` / `rate_limit` / named matchers / `import` / `php_fastcgi` / `try_files` / `templates`. |
| Diff preview before save | shipped | `RawWebServerPanel` renders `<DiffPreview oldText newText/>` behind a "Preview diff against the on-disk version" toggle. |
| Multi-file batch validate | shipped | `RawWebServerPanel` tracks `pendingDirty` across files; "Save all" writes every dirty file, runs one validate on the whole tree, and reloads once. Validate-fail restores all backups via the backend. |
| Lint / health hints | shipped | `web_server_lint_hints` runs `apachectl -S` / `caddy adapt --pretty` / `nginx -t -q`. Both `RawWebServerPanel` and `NginxPanel` expose a "Lint" button + `ValidationBanner` showing the captured output. |
| Open in external editor | shipped | `RawWebServerPanel` downloads to a temp file, spawns the user's `$EDITOR`, watches for saves, and runs the backup→write→validate→reload pipeline on each external save. Stop-watcher button cleans up the temp file. |
| Undo/redo on feature toggles | shipped | `RawWebServerPanel` keeps `undoStack` / `redoStack` of dirty-buffer snapshots; toolbar Undo/Redo buttons + Ctrl/Cmd+Z and Ctrl+Shift+Z / Ctrl+Y bindings cover feature toggles, tree edits, and direct edits. |
| Sidebar grouping ("Web Server" / "Database" / "Shell" sections) | blocked-by-spec | Earlier proposal to fold the sidebar into category sections is a cross-cutting UX refactor; PRODUCT-SPEC §4 (right-side ToolStrip ordering) would need to revisit before the implementation lands. |

## Docker panel — Compose

The current Docker panel has a **Projects** tab that label-groups running
containers by `com.docker.compose.project`. The design's **Compose** tab
is YAML-file-oriented (picks a `docker-compose.*.yml`, shows service
replica counts, and exposes file-level actions). Bridging requires:

| Area | Status | Needed Tauri command(s) / notes |
|---|---|---|
| Pick a `docker-compose.*.yml` on the remote host | blocked-by-spec | PRODUCT-SPEC §5.4 forbids reading compose YAML — would need a spec amendment |
| Parse compose file → services/replicas/image | blocked-by-spec | Same — no YAML parse per spec |
| `docker compose up -d` / `down` / `restart` / `pull` / `build` from the panel | blocked-by-spec | Same — no `docker compose` subprocess per spec |
| Per-service `logs` / `restart` / `stop` actions in the services table | shipped | `feat/docker-compose-derived` — `serviceAction` fans out the existing container commands across replicas (no compose CLI) |
| Health / replica summary (e.g. "4/5 healthy") | shipped | `feat/docker-compose-derived` — service header row carries `{count} replicas` + `{running}/{total} running` |
| "Active compose file" as tab state (remembered per SSH host) | blocked-by-spec | Same — file picking is out-of-spec |

## AI assistant panel (§5.14)

v1 shipped: panel + BYOK providers (Anthropic / OpenAI-compatible / Ollama,
streaming via `pier-core::services::ai::provider` over `ureq`), risk classifier
L0–L3 with the seven red lines + fail-closed default (`risk.rs`, unit-tested),
secret redactor, per-tab conversations with transcript replay
(`ai-history/<tab>.jsonl`) + a memory-only toggle (`aiPersistHistory`),
approval cards (allow once / session / always), host-scoped allowlist
(`ai-whitelist.json`, L1-only — enforced backend-side), tools `run_command` /
`read_file` / `list_dir` / `monitor_snapshot` / `write_file` (SFTP/local
write, 5 MB cap, path classified via `classify_write_path` — critical system
files / audit logs / block devices are L3), terminal link (context-menu "Ask
AI" for selection / visible screen → removable attachment chips; fenced code
blocks in answers get copy + insert-into-terminal without trailing newline,
multi-line only under bracketed paste else clipboard fallback), settings page
(key → keyring `pier-x.ai.<kind>`), `Ctrl+Shift+A`.

| Area | Design surface (§5.14) | Status | Notes |
|---|---|---|---|
| Context attach | Smart-mode block / SFTP editor file as explicit attachments | partial | Terminal selection + visible screen shipped; smart-mode block hover needs a block UI in the webview terminal; SFTP editor "attach current file" not wired |
| NL→command | `#` prefix in terminal input (candidate) | hidden | Spec says decide on usage data |
| Tools | `db_query` (dual-gated with panel write unlock), `git_*`, `docker_*` | hidden | §5.14.3 table requires re-registering each tool as it ships |
| L2 confirm | Typed object-name unlock for data-destroying subset | partial | v1 requires typing the command's first word for ALL L2 — stricter than spec minimum, revisit when `db_query` lands |
| Token usage | Per-turn usage display | partial | Sums shown in panel bar; OpenAI-compatible streams without `stream_options` report no usage (provider-dependent) |
| Local exec cancel | Stop button kills in-flight LOCAL command | done | SSH and local both cancel via `CancellationToken`; `local_exec::exec_cancellable` polls the token and kills the child process on stop |
