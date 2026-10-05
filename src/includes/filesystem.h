#ifndef POPCORN_FILESYSTEM_H
#define POPCORN_FILESYSTEM_H

#include <stdbool.h>

/* FAT32 on the selected block disk (Rust). */

void init_filesystem(void);
bool write_file(const char* name, const char* content);
const char* read_file(const char* name);
void list_files(void);
void list_files_console(void);
bool delete_file(const char* name);
bool create_directory(const char* name);
bool change_directory(const char* name);
const char* get_current_directory(void);
const char* search_file(const char* name);
bool copy_file(const char* src_name, const char* dest_path);
void list_hierarchy(void);
int get_last_filesystem_error(void);

#endif /* POPCORN_FILESYSTEM_H */
