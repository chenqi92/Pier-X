/*
 * pier-core — SSH port forwarding C ABI
 * ──────────────────────────────────────
 *
 * One opaque PierTunnel* per open local port forward. Each
 * handle owns its own SshSession + accept loop + bound local
 * TCP listener. Dropping the handle via pier_tunnel_free
 * stops accepting new connections, releases the local port,
 * and closes the SSH session.
 *
 * Threading: pier_tunnel_open is blocking (SSH handshake).
 * Once it returns, the accept loop runs on pier-core's
 * shared tokio runtime — the C++ side just sees a handle
 * that's "alive" until freed.
 *
 * Local port semantics:
 *   * local_port = 0  → OS picks a free port. Actual port
 *                        available via pier_tunnel_local_port.
 *   * local_port > 0  → Pier-X tries to bind exactly that
 *                        port. Returns NULL if it's in use.
 *
 * Auth kind: same table as pier_sftp.h / pier_services.h.
 */

#ifndef PIER_TUNNEL_H
#define PIER_TUNNEL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PierTunnel PierTunnel;

/* Open a local port forward. Returns NULL on failure. */
PierTunnel *pier_tunnel_open(
    const char *host,
    uint16_t port,
    const char *user,
    int32_t auth_kind,
    const char *secret,       /* NULL for AUTH_AGENT */
    const char *extra,        /* NULL unless AUTH_KEY passphrase */
    uint16_t local_port,      /* 0 = OS picks */
    const char *remote_host,  /* e.g. "127.0.0.1" */
    uint16_t remote_port
);

/* M3e: open a local port forward on an existing shared SSH
 * session (see pier_ssh_session.h). No auth parameters —
 * the session is pre-authenticated. Returns NULL on any
 * failure. */
struct PierSshSession;
PierTunnel *pier_tunnel_open_on_session(
    const struct PierSshSession *session,
    uint16_t local_port,       /* 0 = OS picks */
    const char *remote_host,   /* e.g. "127.0.0.1" */
    uint16_t remote_port
);

/* Return the port the listener is actually bound to (same
 * value that was passed in unless local_port was 0). */
uint16_t pier_tunnel_local_port(const PierTunnel *t);

/* Return 1 if the accept loop is still running, 0 otherwise. */
int32_t pier_tunnel_is_alive(const PierTunnel *t);

/* Close the tunnel. Safe to call with NULL. */
void pier_tunnel_free(PierTunnel *t);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_TUNNEL_H */
