/*
 * pier-core — Docker panel C ABI
 * ──────────────────────────────
 *
 * M5c per-service tool. Thin wrapper around
 * `pier_core::services::docker` — every operation runs a
 * one-shot `docker <verb>` over SSH exec on the handle's
 * session.
 *
 * Threading: every function is synchronous and blocking. The
 * C++ wrapper runs them on a worker thread and posts results
 * back via QMetaObject::invokeMethod, matching the pattern
 * used by PierSftp / PierRedis / PierLogStream.
 *
 * Live logs: `docker logs -f <id>` belongs in the Log viewer
 * stream, not this header. Open a LogViewerView tab with the
 * command string `"docker logs -f --tail 500 <id>"` to see
 * live output for a container.
 *
 * Shell safety: every container id passed to the action
 * functions is validated against a strict allowlist
 * (`[A-Za-z0-9][A-Za-z0-9_.-]{0,254}`) before being
 * interpolated into the remote command string. Non-matching
 * ids are rejected with PIER_DOCKER_ERR_UNSAFE_ID.
 *
 * Auth kind discriminator — same table as every other
 * session-based FFI (pier_sftp.h, pier_tunnel.h, etc).
 *
 * Memory ownership:
 *   * PierDocker * must be released via pier_docker_free.
 *   * JSON strings returned by pier_docker_list_containers
 *     are owned by Rust and must be released via
 *     pier_docker_free_string.
 */

#ifndef PIER_DOCKER_H
#define PIER_DOCKER_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle. */
typedef struct PierDocker PierDocker;

/* Return codes for the action functions (pier_docker_start /
 * stop / restart / remove). */
#define PIER_DOCKER_OK               0
#define PIER_DOCKER_ERR_NULL        -1
#define PIER_DOCKER_ERR_UTF8        -2
#define PIER_DOCKER_ERR_FAILED      -3
#define PIER_DOCKER_ERR_UNSAFE_ID   -4

/* Open a Docker panel. Runs the SSH handshake synchronously
 * and returns NULL on any failure. */
PierDocker *pier_docker_open(
    const char *host,
    uint16_t port,
    const char *user,
    int32_t auth_kind,
    const char *secret,   /* NULL allowed for AUTH_AGENT */
    const char *extra     /* NULL unless AUTH_KEY passphrase */
);

/* M3e: open a Docker panel on an existing shared SSH
 * session (see pier_ssh_session.h). No auth parameters —
 * the session is pre-authenticated. The panel clones the
 * session and drives every subsequent `docker <verb>` exec
 * through it. */
struct PierSshSession;
PierDocker *pier_docker_open_on_session(const struct PierSshSession *session);

/* Release a Docker handle. Safe on NULL. */
void pier_docker_free(PierDocker *h);

/* Release a heap JSON string returned by this module. Safe
 * on NULL. Do NOT use C free(). */
void pier_docker_free_string(char *s);

/* List containers. Returns a heap JSON array of Container
 * objects or NULL on failure. Pass all != 0 to include
 * stopped containers (`docker ps -a`).
 *
 * Container schema (fields may be empty strings if docker
 * didn't report them):
 *   { "id":       string (short or full id),
 *     "image":    string,
 *     "names":    string,
 *     "status":   string ("Up 5m" / "Exited (0) 1h ago"),
 *     "state":    string ("running" | "exited" | ...),
 *     "created":  string,
 *     "ports":    string }
 *
 * Release with pier_docker_free_string. */
char *pier_docker_list_containers(PierDocker *h, int32_t all);

/* Inspect a single container. Returns the raw JSON array
 * emitted by `docker inspect --type container <id>`, or
 * NULL on failure. Release with pier_docker_free_string. */
char *pier_docker_inspect_container(PierDocker *h, const char *id);

/* Execute `docker <args...>` where `args_json` is a JSON
 * array of strings. Returns
 * `{ "ok": bool, "exit_code": number, "output": string }`
 * as a heap string, or NULL on invalid input / transport
 * failure. Release with pier_docker_free_string. */
char *pier_docker_exec_json(PierDocker *h, const char *args_json);

/* Start a container by id. Returns 0 on success or one of
 * the PIER_DOCKER_ERR_* codes. */
int32_t pier_docker_start(PierDocker *h, const char *id);

/* Stop a running container by id. */
int32_t pier_docker_stop(PierDocker *h, const char *id);

/* Restart a container (stop+start) by id. */
int32_t pier_docker_restart(PierDocker *h, const char *id);

/* Remove a container by id. force != 0 passes --force, which
 * also kills running containers — the UI should always
 * confirm before passing force. */
int32_t pier_docker_remove(PierDocker *h, const char *id, int32_t force);

/* ── Images ────────────────────────────────────────────── */

/* List images. Returns JSON array or NULL. Caller frees. */
char *pier_docker_list_images(PierDocker *h);

/* Remove an image by id. Returns 0 or PIER_DOCKER_ERR_*. */
int32_t pier_docker_remove_image(PierDocker *h, const char *id, int32_t force);

/* ── Volumes ───────────────────────────────────────────── */

/* List volumes. Returns JSON array or NULL. Caller frees. */
char *pier_docker_list_volumes(PierDocker *h);

/* Remove a volume by name. Returns 0 or PIER_DOCKER_ERR_*. */
int32_t pier_docker_remove_volume(PierDocker *h, const char *name);

/* ── Networks ──────────────────────────────────────────── */

/* List networks. Returns JSON array or NULL. Caller frees. */
char *pier_docker_list_networks(PierDocker *h);

/* Remove a network by name. Returns 0 or PIER_DOCKER_ERR_*. */
int32_t pier_docker_remove_network(PierDocker *h, const char *name);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_DOCKER_H */
