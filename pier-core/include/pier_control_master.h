/*
 * pier-core — SSH ControlMaster C ABI
 *
 * Execute remote commands through an SSH ControlMaster socket,
 * sharing the terminal's SSH connection without opening a new one.
 */

#ifndef PIER_CONTROL_MASTER_H
#define PIER_CONTROL_MASTER_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PierControlMaster PierControlMaster;

PierControlMaster *pier_control_master_new(const char *host, uint16_t port, const char *user);
int32_t pier_control_master_connect(PierControlMaster *h, uint32_t timeout_secs);
char *pier_control_master_exec(const PierControlMaster *h, const char *command);
int32_t pier_control_master_is_alive(const PierControlMaster *h);
void pier_control_master_free(PierControlMaster *h);
void pier_control_master_free_string(char *s);

#ifdef __cplusplus
}
#endif

#endif /* PIER_CONTROL_MASTER_H */
