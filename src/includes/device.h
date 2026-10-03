#ifndef DEVICE_H
#define DEVICE_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define DEV_NAME_MAX 32
#define MAX_DEVICES  16
#define FD_MAX       16

struct Device;

typedef struct DeviceOps {
    int64_t (*read)(struct Device* dev, void* buf, size_t count);
    int64_t (*write)(struct Device* dev, const void* buf, size_t count);
    int64_t (*ioctl)(struct Device* dev, uint64_t request, void* argp);
} DeviceOps;

typedef struct Device {
    char name[DEV_NAME_MAX];
    const DeviceOps* ops;
    void* priv;
    bool used;
} Device;

typedef struct FileDesc {
    Device* dev;
    bool open;
    uint32_t flags;
} FileDesc;

void device_init(void);
Device* device_register(const char* name, const DeviceOps* ops, void* priv);
void device_register_rust(const char* name);
Device* device_find(const char* name);

/* Per-task fd helpers (table lives on TaskStruct). */
void fd_table_init(FileDesc* fds);
int fd_alloc(FileDesc* fds, Device* dev, uint32_t flags);
Device* fd_get(FileDesc* fds, int fd);
int fd_close(FileDesc* fds, int fd);

#endif /* DEVICE_H */
