/*
 * pier-core — C ABI header
 * ─────────────────────────
 *
 * This header declares the stable C ABI exported by the `pier-core` Rust
 * static library. Any C or C++ consumer (today: a thin pier-ui-qt C++
 * wrapper that exposes these as a QML singleton) links against the
 * staticlib and includes this header.
 *
 * Memory contract
 *   - Every `const char*` returned is statically allocated inside pier-core
 *     and valid for the lifetime of the process. Callers must NOT free them.
 *   - Inputs are NUL-terminated UTF-8. Passing NULL is defined and returns
 *     the same result as an empty string.
 *
 * When this file grows beyond ~20 functions it will be replaced by a
 * cbindgen-generated header. For the M1 smoke-test surface (three
 * functions) a hand-written header keeps the build graph trivial.
 */

#ifndef PIER_CORE_H
#define PIER_CORE_H

#ifdef __cplusplus
extern "C" {
#endif

/* Returns the pier-core crate version (e.g. "0.1.0"). */
const char* pier_core_version(void);

/* Returns "<version> (release|debug)" for display in About / status bars. */
const char* pier_core_build_info(void);

/* Returns 1 if pier-core was built with the named feature, 0 otherwise.
 * Recognised names will grow with each protocol module port. NULL-safe. */
int pier_core_has_feature(const char* name);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_CORE_H */
