#include "../includes/device.h"
#include "../includes/console.h"
#include "../includes/utils.h"
#include <stddef.h>

static Device g_devices[MAX_DEVICES];
static uint32_t g_device_count;

static void name_copy(char* dst, const char* src, size_t cap) {
    size_t i = 0;
    if (!dst || cap == 0) {
        return;
    }
    if (src) {
        for (; i + 1 < cap && src[i]; i++) {
            dst[i] = src[i];
        }
    }
    dst[i] = '\0';
}

static int name_eq(const char* a, const char* b) {
    if (!a || !b) {
        return 0;
    }
    while (*a && *b && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

/* Strip optional "/dev/" prefix for lookups. */
static const char* strip_dev_prefix(const char* name) {
    if (!name) {
        return "";
    }
    if (name[0] == '/' && name[1] == 'd' && name[2] == 'e' && name[3] == 'v' && name[4] == '/') {
        return name + 5;
    }
    return name;
}

static int64_t console_read(Device* dev, void* buf, size_t count) {
    (void)dev;
    (void)buf;
    (void)count;
    /* Non-blocking empty for now; blocking reads use wait queues in syscall. */
    return 0;
}

static int64_t console_write(Device* dev, const void* buf, size_t count) {
    (void)dev;
    if (!buf) {
        return -2; /* EINVAL */
    }
    if (count > 4096) {
        return -2;
    }
    const char* s = (const char*)buf;
    for (size_t i = 0; i < count; i++) {
        console_putchar(s[i]);
    }
    return (int64_t)count;
}

static int64_t console_ioctl(Device* dev, uint64_t request, void* argp) {
    (void)dev;
    switch (request) {
        case 0x5401: /* TCGETS */
        case 0x5402: /* TCSETS */
            return 0;
        case 0x540B: /* TIOCGWINSZ */
            if (argp) {
                uint16_t* winsize = (uint16_t*)argp;
                winsize[0] = 80;
                winsize[1] = 25;
                winsize[2] = 0;
                winsize[3] = 0;
            }
            return 0;
        default:
            return -2;
    }
}

static const DeviceOps console_ops = {
    .read = console_read,
    .write = console_write,
    .ioctl = console_ioctl,
};

static int64_t null_read(Device* dev, void* buf, size_t count) {
    (void)dev;
    (void)buf;
    (void)count;
    return 0;
}

static int64_t null_write(Device* dev, const void* buf, size_t count) {
    (void)dev;
    (void)buf;
    return (int64_t)count;
}

static int64_t null_ioctl(Device* dev, uint64_t request, void* argp) {
    (void)dev;
    (void)request;
    (void)argp;
    return -2;
}

static const DeviceOps null_ops = {
    .read = null_read,
    .write = null_write,
    .ioctl = null_ioctl,
};

void device_init(void) {
    for (uint32_t i = 0; i < MAX_DEVICES; i++) {
        g_devices[i].used = false;
        g_devices[i].ops = NULL;
        g_devices[i].priv = NULL;
        g_devices[i].name[0] = '\0';
    }
    g_device_count = 0;
    device_register("console", &console_ops, NULL);
    device_register("null", &null_ops, NULL);
}

Device* device_register(const char* name, const DeviceOps* ops, void* priv) {
    if (!name || !ops || g_device_count >= MAX_DEVICES) {
        return NULL;
    }
    const char* n = strip_dev_prefix(name);
    for (uint32_t i = 0; i < MAX_DEVICES; i++) {
        if (!g_devices[i].used) {
            g_devices[i].used = true;
            name_copy(g_devices[i].name, n, DEV_NAME_MAX);
            g_devices[i].ops = ops;
            g_devices[i].priv = priv;
            g_device_count++;
            return &g_devices[i];
        }
    }
    return NULL;
}

Device* device_find(const char* name) {
    const char* n = strip_dev_prefix(name);
    for (uint32_t i = 0; i < MAX_DEVICES; i++) {
        if (g_devices[i].used && name_eq(g_devices[i].name, n)) {
            return &g_devices[i];
        }
    }
    return NULL;
}

void fd_table_init(FileDesc* fds) {
    if (!fds) {
        return;
    }
    for (int i = 0; i < FD_MAX; i++) {
        fds[i].dev = NULL;
        fds[i].open = false;
        fds[i].flags = 0;
    }
    Device* cons = device_find("console");
    if (cons) {
        for (int i = 0; i < 3; i++) {
            fds[i].dev = cons;
            fds[i].open = true;
            fds[i].flags = 0;
        }
    }
}

int fd_alloc(FileDesc* fds, Device* dev, uint32_t flags) {
    if (!fds || !dev) {
        return -1;
    }
    for (int i = 0; i < FD_MAX; i++) {
        if (!fds[i].open) {
            fds[i].open = true;
            fds[i].dev = dev;
            fds[i].flags = flags;
            return i;
        }
    }
    return -1;
}

Device* fd_get(FileDesc* fds, int fd) {
    if (!fds || fd < 0 || fd >= FD_MAX || !fds[fd].open) {
        return NULL;
    }
    return fds[fd].dev;
}

int fd_close(FileDesc* fds, int fd) {
    if (!fds || fd < 0 || fd >= FD_MAX || !fds[fd].open) {
        return -1;
    }
    fds[fd].open = false;
    fds[fd].dev = NULL;
    fds[fd].flags = 0;
    return 0;
}
