#ifndef PIER_SEARCH_H
#define PIER_SEARCH_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Search files by name pattern. Returns JSON array. Caller frees. */
char *pier_search_files(const char *root, const char *pattern, uint32_t max_results);

/* Search file contents. Returns JSON array. Caller frees. */
char *pier_search_content(const char *root, const char *pattern, uint32_t max_results);

void pier_search_free_string(char *s);

#ifdef __cplusplus
}
#endif

#endif
