/*
 * pier-core — Local execution C ABI
 * ──────────────────────────────────
 * Run Docker, system metrics, and shell commands locally
 * without SSH. All functions are synchronous and blocking.
 *
 * Memory: char* returns must be freed via pier_local_free_string.
 */

#ifndef PIER_LOCAL_H
#define PIER_LOCAL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

void pier_local_free_string(char *s);

/* ── Local Docker ──────────────────────────────────────── */
char *pier_local_docker_list_containers(int32_t all);
char *pier_local_docker_list_images(void);
char *pier_local_docker_list_volumes(void);
char *pier_local_docker_list_networks(void);
char *pier_local_docker_exec_json(const char *args_json);
int32_t pier_local_docker_action(const char *verb, const char *id, int32_t force);
char *pier_local_docker_inspect(const char *id);

/* ── Local System Metrics ──────────────────────────────── */
char *pier_local_system_metrics(void);

/* ── Local Shell ───────────────────────────────────────── */
char *pier_local_exec(const char *cmd);

#ifdef __cplusplus
}
#endif

#endif /* PIER_LOCAL_H */
