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
| `feat/terminal-history-persistence` | Smart Mode terminal: per-shell history persisted to `~/.pier-x/terminal-history-<shell>.jsonl` |
| `feat/web-server-unify` | Web Server panel consolidates nginx/Apache/Caddy under one `rightTool: "webserver"`. Detection (`web_server_detect`), generic validate/reload (`web_server_validate`/`_reload`), shared layout/read/save pipeline (`web_server_layout`/`_read_file`/`_save_file`), Apache site toggle (`web_server_toggle_site`), new-site wizard (`web_server_create_site`), Caddy parser/renderer (`caddy_parse`/`_render`, 5 tests), Apache parser/renderer (`apache_parse`/`_render`, 7 tests). Apache catalog 9 features, Caddy catalog 9 features. |

## MySQL panel

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Splash | "Probe via {ssh target}" activity line with Re-probe button | partial | `dbDetect` already exists; button wires to `refreshDetection` |
| Splash | Instance row meta: `engine`, `addr`, `via`, `user`, `authFrom`, `lastUsed`, `dbs`, `size` | partial | Saved creds expose `{host,port,user,database}`; `engine`, `authFrom`, `lastUsed`, `dbs`, `size` not stored — add to `DbCredential`/detection |
| Splash | `prod / stage / dev / local` env tag per instance | stub | Needs a `env` / `tag` field on saved credentials (user-editable) |
| Header | Stats chips: `{dbs} dbs`, `{size}`, `{ms} roundtrip` | partial | `dbs` = `state.databases.length`; `size` and `ms` roundtrip not measured |
| Schema tree | Views, Functions under a schema | shipped | `feat/mysql-schema-enrichment` — `mysqlBrowse` now returns views + routines |
| Schema tree | Row count per table | shipped | `feat/mysql-schema-enrichment` — table-meta tooltip carries `table_rows` |
| Data tab | Column width resize grip | hidden | Pure frontend (when we add per-column width state) |
| Data tab | Per-column filter row | stub | Frontend-only filter against already-loaded preview rows |
| Data tab | Sort indicator on header | stub | Same — runs against loaded rows, not a server sort |
| Data tab | Inline CRUD (edit / insert / delete with pending commit batch) | shipped | `feat/db-grid-crud` — `DbResultGrid` collects pending mutations, `mutationToSql` builds quoted UPDATE/INSERT/DELETE per dialect, single Commit button fans them through `mysqlExecute` (works for both MySQL and PG) |
| Data tab | Server-side paging (page N of M) | shipped | `feat/mysql-paging-history` — `mysql_browse(offset, limit)` + `total_rows`; pager + page-size dropdown in toolbar |
| Data tab | Elapsed `ms` on grid toolbar | hidden | Only `queryResult.elapsedMs` exists, not a per-browse number |
| SQL editor | Multiple query tabs | stub | Pure frontend state (add later, no backend) |
| SQL editor | History drawer (recent queries + status) | shipped | `feat/mysql-paging-history` — `useDbSqlTabs` persists per-engine to localStorage (`pier-x:sql-history:<engine>`), 200-entry cap |
| SQL editor | Favorites | shipped | `useDbSqlTabs` persists pinned queries per engine in `pier-x:sql-favorites:<engine>` (50-entry cap); editor exposes Add/Remove/Pick from the rail |
| SQL editor | Format SQL button | shipped | `feat/sql-explain-format` — sql-formatter dep + button on all three SQL panels |
| SQL editor | EXPLAIN button | shipped | `feat/sql-explain-format` — runs `EXPLAIN <sql>` via existing execute |
| SQL editor | EXPLAIN ANALYZE plan tree | shipped | `feat/explain-plan-tree` — `Plan` button runs `EXPLAIN (ANALYZE, FORMAT JSON, BUFFERS)` (PG) or `EXPLAIN FORMAT=JSON` (MySQL); JSON parsed by `lib/explainPlan.ts` into a unified `PlanNode`; rendered hierarchically by `ExplainPlanView` with rows/cost/actual-time/buffers chips |
| Row detail | Foreign-key "X (N) →" links | partial | `feat/db-structure-keys` ships the underlying FK metadata; row-detail navigation still pending |
| Structure tab | Columns / Indexes / Foreign keys tables | shipped | `feat/db-structure-keys` — indexes + FK sections under the column grid |
| Schema tab | Per-table engine / rows / data / idx / updated | shipped | `feat/mysql-schema-enrichment` — table-meta tooltip exposes engine / rows / size |

## PostgreSQL panel

Mirrors MySQL, with the following PG-specific gaps on top:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Schema tree | Schemas under a database (left-rail `public` / `reporting` / …) | shipped | `feat/pg-schema-picker-pool` — `postgresBrowse` returns `schemas[]`; tree renders schema picker |
| Schema tree | Views and routines (functions, procedures) | shipped | `feat/pg-schema-enrichment` — views + routines listed per schema |
| Header stats | Connection pool / backend count | shipped | `feat/pg-schema-picker-pool` — `pool_status` walks `pg_stat_activity`; chip shows `{active}/{total}` |
| Row detail | `pg_catalog` type decoration (e.g. `shipment_status[]`) | partial | Column types today come through as raw strings; acceptable for MVP |
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
| Scan | Pattern + DB change without running the full browse | partial | `redisBrowse` already does this — UI just needs to feed the new values |
| CLI | Rich REPL (history, up-arrow recall) | stub | Current impl runs a single statement; history belongs in frontend state |

## SQLite panel

The SQLite panel already has more backend coverage than the design — it
adds a remote capability probe and scan-directory flow the design doesn't
have. Remaining visual / data gaps:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Splash — saved profiles | Persisted SQLite file paths as reusable profiles | hidden | Blocked: `dbCredSave` keys to an SSH connection_index; local SQLite has no anchor. Closing this needs a new "global" credential bucket — out of scope for `feat/sqlite-cluster` |
| Splash — env tag | `local` vs `prod · remote read` tag | stub | Frontend-only once saved profiles land |
| Structure tab | Indexes + triggers per table | shipped | `feat/sqlite-cluster` — `PRAGMA index_list` / `index_info` / `sqlite_master` triggers; rendered under the column grid |
| Connected header | Rough "{size}" stat for the opened file | shipped | `feat/sqlite-cluster` — `std::fs::metadata` locally; `stat -c %s ‖ stat -f %z` over SSH |
| Query editor | Multi-statement scripts + per-statement timing | shipped | `feat/sqlite-cluster` — new `sqlite_execute_script` splits on top-level `;` (quote/comment-aware) and returns per-statement timing |

## SFTP file editor dialog

Visual chrome is now design-matched (header chips + View/Edit segment +
toolbar + footer). Functional gaps from the design:

| Area | Design surface | Status | Needed Tauri command(s) / notes |
|---|---|---|---|
| Header chips | `owner` chip (e.g. `deploy:deploy`) | shipped | `feat/sftp-cluster` — `RemoteFileEntry` carries `owner` / `group` (named, falling back to numeric); rendered as a head chip + browser column |
| Toolbar | Dedicated Find vs Find-and-Replace buttons | partial | The design opens an in-dialog bar; we defer to CodeMirror6's built-in panel via the same `Search` icon — acceptable but slightly different look |
| Toolbar | Download to local disk | stub | Implemented frontend-only (in-memory blob → <a download>) — sandbox environments may block it |
| Toolbar | Copy path | stub | Frontend-only via clipboard — done |
| Footer | EOL detection (LF / CRLF) | shipped | `feat/sftp-cluster` — `detect_eol` walks the decoded text once, picks the dominant kind, ties → "mixed" |
| Footer | Encoding detection beyond UTF-8 vs lossy | shipped | `feat/sftp-cluster` — 3-byte BOM + NUL-scan classifier, surfaces utf-8 / utf-8-bom / utf-16-le / utf-16-be / binary |
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
| Left rail — time range chips (1m / 15m / 1h / 24h / all) | partial | `feat/log-viewer-cluster` — chips filter the live ring client-side. True back-fill (`journalctl --since`, `tail -n`) still pending |
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
| Apache feature catalog beyond 9 | partial | Shipped: identity / TLS / proxy / alias / rewrite / headers / auth / directory / logging. Not yet: `<IfModule>` conditional editor, `LimitRequestBody`, `Timeout`/`KeepAlive`, MPM tuning (StartServers / MaxRequestWorkers), `<RequireAll>` / `<RequireAny>`, `mod_deflate`, `mod_expires`, `Listen`, `ServerTokens` / `ServerSignature`. Each ~60-80 LOC. |
| Caddy feature catalog beyond 9 | partial | Shipped: tls / reverse_proxy / file_server / encode / headers / basicauth / rewrite / redir / log. Not yet: `handle_path` / `handle` route grouping, `rate_limit`, named matchers (`@matcher`) editor, `import` smart manager, `php_fastcgi`, `try_files`, `templates`, global options block (acme_dns / debug / admin / order). |
| Diff preview before save | hidden | Show backup → new diff in a `<details>` before the user commits, so prod edits get a sanity-check window. Reuse existing diff infrastructure. |
| Multi-file batch validate | hidden | Apache vhost edits land in separate `sites-enabled/*` files but `apachectl configtest` covers the whole tree — show a per-file dirty-set + one-shot save-and-validate-all flow. |
| Lint / health hints | hidden | Run `apachectl -S` / `caddy adapt --pretty` after save to surface server-detected warnings inside the panel (e.g. duplicate ServerName, fall-through routes). |
| Open in external editor | hidden | Cross-product feature: temp-file download → spawn user's `$EDITOR` → watch for save → upload-back. Reuse the SFTP panel's existing watcher. |
| Undo/redo on feature toggles | hidden | Card toggles have no undo — accidental clicks need Raw mode to reverse. Add a stack of dirty-buffer snapshots scoped to the panel session. |
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
