#ifndef DRIVER_ABI_H
#define DRIVER_ABI_H

#include <stdint.h>
#include <stddef.h>

/* Hardware drives (Rust): start them, talk to them, list /dev nodes. */

/* Boot: start the built-in set (null, zero, serial, screen). */
void init_drives(void);

/* Start one drive by name (e.g. "null", "ttyS0", "tty0"). Returns 0 on success. */
int init_drive(const char* name);

/* List known drives / active /dev nodes into a buffer (NUL-terminated). */
int list_drives(char* buf, size_t buflen);
int list_devices(char* buf, size_t buflen);

/* Ask a drive something: "status", "info", "init". Reply written to buf. */
int drive_cmd(const char* target, const char* cmd, char* buf, size_t buflen);

/* Named device I/O used by the C /dev table bridge. */
int64_t rust_device_read(const char* name, void* buf, size_t count);
int64_t rust_device_write(const char* name, const void* buf, size_t count);
int64_t rust_device_ioctl(const char* name, uint64_t request, void* argp);

/* Display drive — console/shell call these to draw (Rust owns VGA/GOP). */
void rust_screen_init_vga(void);
int rust_screen_init_fb(uint64_t addr, uint32_t pitch, uint32_t width, uint32_t height,
                        uint8_t bpp, uint8_t type, uint8_t red_pos, uint8_t red_size,
                        uint8_t green_pos, uint8_t green_size, uint8_t blue_pos,
                        uint8_t blue_size); /* 1=fb, 0=vga fallback */
char* rust_screen_cells(void);
int rust_screen_backend(void); /* 0=none 1=vga 2=fb */
uint32_t rust_screen_cols(void);
uint32_t rust_screen_rows(void);
void rust_screen_set_cursor(uint32_t x, uint32_t y);
void rust_screen_set_cursor_visible(int visible);
void rust_screen_write_cell(uint32_t x, uint32_t y, uint8_t ch, uint8_t attr);
void rust_screen_present(void);
void rust_screen_sync_begin(void);
void rust_screen_sync_end(void);
void rust_screen_mark_row(uint32_t y);
void rust_screen_clear(uint8_t attr);
void rust_screen_paint_bg(uint32_t rgb);
void rust_screen_relayout(void);
void rust_screen_fill_panel(void);
void rust_screen_invalidate(void);

void device_register_rust(const char* name);

#endif /* DRIVER_ABI_H */
