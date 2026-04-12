/*
 * pier-core — Server resource monitor C ABI (M7b)
 *
 * Run `uptime + free + df + /proc/stat` over SSH exec and
 * return a JSON snapshot of CPU/RAM/disk/uptime. The C++
 * side polls pier_server_monitor_probe on a QTimer.
 *
 * JSON shape (ServerSnapshot):
 *   { "uptime": "up 5 days, 3:42",
 *     "load_1": 0.12, "load_5": 0.34, "load_15": 0.56,
 *     "mem_total_mb": 16000, "mem_used_mb": 8000, "mem_free_mb": 8000,
 *     "swap_total_mb": 2048, "swap_used_mb": 100,
 *     "disk_total": "100G", "disk_used": "40G",
 *     "disk_avail": "55G", "disk_use_pct": 42,
 *     "cpu_pct": 23.5 }
 *
 * Fields at -1 mean "not available" (e.g. cpu_pct on macOS).
 */

#ifndef PIER_SERVER_MONITOR_H
#define PIER_SERVER_MONITOR_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PierServerMonitor PierServerMonitor;

PierServerMonitor *pier_server_monitor_open(
    const char *host, uint16_t port, const char *user,
    int32_t auth_kind, const char *secret, const char *extra);

struct PierSshSession;
PierServerMonitor *pier_server_monitor_open_on_session(
    const struct PierSshSession *session);

char *pier_server_monitor_probe(PierServerMonitor *h);
void pier_server_monitor_free_string(char *s);
void pier_server_monitor_free(PierServerMonitor *h);

#ifdef __cplusplus
}
#endif

#endif /* PIER_SERVER_MONITOR_H */
