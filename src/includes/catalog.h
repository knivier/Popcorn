#ifndef POPCORN_CATALOG_H
#define POPCORN_CATALOG_H

#include <stdint.h>
#include <stddef.h>

/* In-kernel registry database (RAM). Implemented in Rust `catalog.rs`. */

#define CATALOG_KIND_ALL      0
#define CATALOG_KIND_DRIVE    1
#define CATALOG_KIND_DEVICE   2
#define CATALOG_KIND_POP      3
#define CATALOG_KIND_IRQ      4
#define CATALOG_KIND_SYSCALL  5

#define CATALOG_STATE_IDLE    0
#define CATALOG_STATE_READY   1
#define CATALOG_STATE_BOUND   2

typedef struct {
    uint8_t kind;
    uint8_t state;
    uint16_t reserved;
    uint32_t id;
    char name[32];
    char class_name[16];
} CatalogEntry;

void rust_catalog_init(void);
uint32_t rust_catalog_count(void);
int rust_catalog_list(uint8_t kind, char* buf, size_t buflen);
int rust_catalog_lookup(const char* name, CatalogEntry* out);

#endif /* POPCORN_CATALOG_H */
