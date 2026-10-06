#ifndef POPCORN_DISK_H
#define POPCORN_DISK_H

#include <stdint.h>
#include <stddef.h>

/* Write-gated block disks (Rust).
 * Boot/USB auto-selected; internal/ESP locked until disk install … YES.
 * disk wipe … YES quick-wipes head/tail and formats Popcorn FAT32. */

#define DISK_CLASS_RAM      1
#define DISK_CLASS_VIRTIO   2
#define DISK_CLASS_USB      3
#define DISK_CLASS_INTERNAL 4

void rust_disk_init(void);
int rust_disk_list(char* buf, size_t buflen);
int rust_disk_use(const char* name);
/* 0=unlocked+selected, 1=armed (type YES), -1=error */
int rust_disk_install(const char* name, int yes);
/* 0=wiped+FAT32, 1=armed (type YES), -1=error */
int rust_disk_wipe(const char* name, int yes);
int rust_disk_info(char* buf, size_t buflen);
/* Read/write one sector on the *selected* disk. Write fails if none/!writable. */
int rust_disk_read(uint64_t lba, void* buf, size_t buflen);
int rust_disk_write(uint64_t lba, const void* buf, size_t buflen);

#endif /* POPCORN_DISK_H */
