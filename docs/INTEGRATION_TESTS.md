# Integration tests

Pier-X's "real service" e2e tests live in [`pier-core/tests/`](../pier-core/tests/)
and are gated behind the Cargo feature `integration-tests`. The default
`cargo test -p pier-core` does **not** run them — they require an
actual MySQL / PostgreSQL server (and, in future phases, sshd / Redis /
Docker daemon).

This file documents how to run them locally and how the CI job is
wired.

---

## Currently covered

| Backend | File | Notes |
|---|---|---|
| MySQL | [`tests/integration_db.rs`](../pier-core/tests/integration_db.rs) | connect / `list_databases` / `SELECT 1` / wrong-password auth error |
| PostgreSQL | same file | mirror of the MySQL set |

Phase B / C add SSH, SFTP, Redis, Docker — same gate, separate test files.

---

## Run locally

### One-time: spin up the test services with Docker

```sh
# MySQL on 3306
docker run -d --name pier-test-mysql \
  -e MYSQL_ROOT_PASSWORD=pier-x-test \
  -p 3306:3306 \
  mysql:8

# PostgreSQL on 5432
docker run -d --name pier-test-pg \
  -e POSTGRES_USER=postgres \
  -e POSTGRES_PASSWORD=pier-x-test \
  -p 5432:5432 \
  postgres:16

# Wait ~15s for both servers to finish initializing.
```

(You can also point the tests at any existing MySQL / Postgres on your
laptop — see `PIER_TEST_*` env vars below.)

### Run the tests

```sh
cargo test -p pier-core \
  --features integration-tests \
  --test integration_db \
  -- --test-threads=1
```

`--test-threads=1` is load-bearing: a single MySQL / Postgres instance
shared across parallel tests can deadlock on connection caps or step
on each other's session state.

### Cleanup

```sh
docker rm -f pier-test-mysql pier-test-pg
```

---

## Environment variables

All defaults match the `docker run` commands above. Override any of
them to point at a different server.

| Var | Default | Used by |
|---|---|---|
| `PIER_TEST_MYSQL_HOST` | `127.0.0.1` | MySQL tests |
| `PIER_TEST_MYSQL_PORT` | `3306` | MySQL tests |
| `PIER_TEST_MYSQL_USER` | `root` | MySQL tests |
| `PIER_TEST_MYSQL_PASSWORD` | `pier-x-test` | MySQL tests |
| `PIER_TEST_MYSQL_DB` | (unset) | MySQL tests — optional default schema |
| `PIER_TEST_PG_HOST` | `127.0.0.1` | PostgreSQL tests |
| `PIER_TEST_PG_PORT` | `5432` | PostgreSQL tests |
| `PIER_TEST_PG_USER` | `postgres` | PostgreSQL tests |
| `PIER_TEST_PG_PASSWORD` | `pier-x-test` | PostgreSQL tests |
| `PIER_TEST_PG_DB` | (unset) | PostgreSQL tests — optional default db |

---

## CI

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml)'s
`integration-tests` job runs on `ubuntu-latest`, uses GHA's native
[`services:`](https://docs.github.com/en/actions/using-containerized-services/about-service-containers)
block to start MySQL 8 + Postgres 16 in sidecar containers, waits
until both are healthy, then runs the same `cargo test` command above.

It is currently `continue-on-error: true` — informational while we
tune flake rates. Once stable for a few weeks, drop the flag to make
it block PR merges alongside `lint` / `rust-core` / `gpui-shell`.

The `services:` approach is Linux-only — macOS / Windows GHA runners
don't have a Docker daemon. That's fine for backend tests (the
network protocols themselves are OS-agnostic); we'll cross that
bridge if a regression turns out to be platform-specific.

---

## Adding a new backend

The pattern from `tests/integration_db.rs`:

1. **Pick a service that GHA can start as a sidecar** (Redis, MariaDB,
   Mongo all qualify). For services that need custom config (sshd
   with a generated host key) you'll write a `Dockerfile` and use
   `docker build` in a CI step instead.
2. **Add env-var-driven config builder** at the top of a new
   `tests/integration_<backend>.rs`:
   ```rust
   #![cfg(feature = "integration-tests")]
   fn redis_config_from_env() -> RedisConfig { /* … */ }
   ```
3. **Cover at least connect + one read + one write + one error path**.
   Anything more belongs in unit tests if pier-core can mock it, or
   in a separate test file if it needs the live server.
4. **Reuse `--test-threads=1`** — same shared-server reasoning.
5. **Update this doc** with the env-var matrix and the docker
   one-liner.
6. **Update `.github/workflows/ci.yml`** to add the service +
   env-var + (if it's a new test file) include it in the `cargo test`
   invocation. Pattern-match on the existing MySQL / PG entries.

---

## Why feature gate, not `#[ignore]`?

`#[ignore]` would still compile the tests on every `cargo test` and
require explicit `--ignored` to run them. The feature flag is
explicit about "this needs an external dependency" — it shows up in
`Cargo.toml`, in CI invocation, and in the file's `#![cfg(...)]`
header. It also lets the lib stay pure for downstream consumers who
don't enable the feature.
