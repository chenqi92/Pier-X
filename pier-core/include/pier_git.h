/*
 * pier-core — Git client C ABI
 * ─────────────────────────────
 *
 * Local-repository Git panel. Unlike MySQL/Redis/Docker this
 * does NOT require an SSH tunnel — it operates directly on the
 * local filesystem's .git directory.
 *
 * Threading: every function is synchronous and blocking. The
 * C++ wrapper runs them on a worker thread and posts results
 * back via QMetaObject::invokeMethod, matching the pattern
 * used by PierMySqlClient / PierRedisClient.
 *
 * Memory ownership:
 *   * PierGit * must be released via pier_git_free.
 *   * char * strings returned by pier_git_status / _diff /
 *     _branch_info / _commit / _push / _pull / _graph_log /
 *     _compute_graph_layout / _list_branches / _list_authors /
 *     _first_parent_chain / _detect_default_branch are owned
 *     by Rust and must be released via pier_git_free_string.
 *     Do NOT use C free().
 */

#ifndef PIER_GIT_H
#define PIER_GIT_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle. */
typedef struct PierGit PierGit;

/* ── Lifecycle ─────────────────────────────────────────── */

/* Open a Git client for the repo at or above `repo_path`.
 * Returns NULL if the path is not inside a Git working tree. */
PierGit *pier_git_open(const char *repo_path);

/* Release the handle. Safe to call with NULL. */
void pier_git_free(PierGit *h);

/* Release a heap string. Safe to call with NULL. */
void pier_git_free_string(char *s);

/* ── Status ────────────────────────────────────────────── */

/* Working tree status as JSON array:
 * [{ "path": "...", "status": "Modified", "staged": true }, ...]
 * Returns { "error": "..." } on failure. Caller frees. */
char *pier_git_status(PierGit *h);

/* ── Diff ──────────────────────────────────────────────── */

/* Unified diff for `path` (NULL/empty = all files).
 * staged=1 for cached diff, staged=0 for working tree.
 * Returns raw diff text. Caller frees. */
char *pier_git_diff(PierGit *h, const char *path, int staged);

/* Diff for an untracked file (full content). Caller frees. */
char *pier_git_diff_untracked(PierGit *h, const char *path);

/* ── Branch info ───────────────────────────────────────── */

/* Current branch + tracking as JSON:
 * { "name": "main", "tracking": "origin/main",
 *   "ahead": 2, "behind": 0 }
 * Caller frees. */
char *pier_git_branch_info(PierGit *h);

/* ── Staging ───────────────────────────────────────────── */

/* Stage files. paths_json = JSON array of path strings.
 * Returns 0 on success, -1 on failure. */
int pier_git_stage(PierGit *h, const char *paths_json);

/* Stage all changes. Returns 0/-1. */
int pier_git_stage_all(PierGit *h);

/* Unstage files. paths_json = JSON array of path strings.
 * Returns 0/-1. */
int pier_git_unstage(PierGit *h, const char *paths_json);

/* Unstage all. Returns 0/-1. */
int pier_git_unstage_all(PierGit *h);

/* Discard working tree changes. paths_json = JSON array.
 * Returns 0/-1. */
int pier_git_discard(PierGit *h, const char *paths_json);

/* ── Commit ────────────────────────────────────────────── */

/* Commit staged changes. Returns commit output or error JSON.
 * Caller frees. */
char *pier_git_commit(PierGit *h, const char *message);

/* ── Remote ────────────────────────────────────────────── */

/* Push current branch. Returns output or error JSON. Caller frees. */
char *pier_git_push(PierGit *h);

/* Pull from remote. Returns output or error JSON. Caller frees. */
char *pier_git_pull(PierGit *h);

/* ── Graph (from git_graph module) ─────────────────────── */

/* Load commit graph with filters. Returns JSON array of
 * CommitEntry objects. Optional params may be NULL.
 * Caller frees. */
char *pier_git_graph_log(
    const char *repo_path,
    uint32_t    limit,
    uint32_t    skip,
    const char *branch,         /* NULL = all refs */
    const char *author,         /* NULL = no filter */
    const char *search_text,    /* NULL = no filter */
    int64_t     after_timestamp,/* 0 = no filter */
    bool        topo_order,
    bool        first_parent,
    bool        no_merges,
    const char *paths           /* NULL or JSON array of paths */
);

/* Compute IDEA-style graph layout from commits JSON.
 * Returns JSON array of GraphRow objects with segments/arrows.
 * Caller frees. */
char *pier_git_compute_graph_layout(
    const char *commits_json,
    const char *main_chain_json,
    float       lane_width,
    float       row_height,
    bool        show_long_edges
);

/* List all branch names. Returns JSON array. Caller frees. */
char *pier_git_list_branches(const char *repo_path);

/* List unique commit authors. Returns JSON array. Caller frees. */
char *pier_git_list_authors(const char *repo_path, uint32_t limit);

/* First-parent chain hashes. Returns JSON array. Caller frees. */
char *pier_git_first_parent_chain(
    const char *repo_path,
    const char *ref_name,
    uint32_t    limit
);

/* Detect default branch (main/master). Returns string. Caller frees. */
char *pier_git_detect_default_branch(const char *repo_path);

#ifdef __cplusplus
}
#endif

#endif /* PIER_GIT_H */
