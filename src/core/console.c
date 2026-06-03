#include "../includes/console.h"
#include "../includes/multiboot2.h"
#include "../includes/boot_fb.h"
#include "../includes/utils.h"
#include "../includes/uefi_input.h"
#include "../includes/timer.h"
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

/* Row above the status bar (status uses VGA_HEIGHT - 1). */
#define HEARTBEAT_ROW (VGA_HEIGHT - 2u)
#define HEARTBEAT_COL 52u

static volatile uint64_t console_alive_seq;

// VGA Hardware Cursor Ports
#define VGA_CTRL_PORT  0x3D4
#define VGA_DATA_PORT  0x3D5
#define VGA_CURSOR_LOC_LOW  0x0F
#define VGA_CURSOR_LOC_HIGH 0x0E

// External console state (defined in core/kernel.c)
extern ConsoleState console_state;

/*
 * Text backend.
 *
 * Legacy VGA text mode (BIOS/CSM machines, QEMU): vga_memory points at the real
 * VGA text buffer at 0xB8000 and writes appear directly.
 *
 * UEFI framebuffer mode (no-CSM machines like ThinkPad T14s Gen 4): there is no
 * VGA text buffer, so vga_memory points at an in-RAM shadow of the 80x25 text
 * cells and console_present() renders changed cells into GRUB's linear
 * framebuffer using an 8x8 bitmap font. All existing console/pop code keeps
 * working unchanged against vga_memory; only the present step differs.
 */
static char* vga_memory = (char*)VGA_MEMORY_ADDRESS;

// ---- Framebuffer text backend ----------------------------------------------

#define FB_FONT_W 8u
#define FB_FONT_H 8u

static volatile uint8_t* fb_base = NULL;
static uint8_t fb_cell_scale = 1u;
static uint32_t fb_pitch = 0;
static uint32_t fb_width = 0;
static uint32_t fb_height = 0;
static uint8_t  fb_bytespp = 0;
static uint8_t  fb_rpos = 0, fb_rsize = 8;
static uint8_t  fb_gpos = 0, fb_gsize = 8;
static uint8_t  fb_bpos = 0, fb_bsize = 8;
static uint32_t fb_origin_x = 0, fb_origin_y = 0;
static bool     fb_active = false;
static unsigned int fb_prev_cursor = 0xFFFFFFFFu;

// Shadow of the 80x25 text cells (char, attr) used while in framebuffer mode.
static char fb_text_shadow[VGA_MEMORY_SIZE];
// Last cells actually drawn to the framebuffer (for dirty-cell diffing).
static char fb_text_rendered[VGA_MEMORY_SIZE];

// 16-color VGA palette -> 0xRRGGBB (index 0 = Popcorn navy, not pure black)
static const uint32_t fb_palette[16] = {
    POPCORN_FB_BG_RGB, 0x0000AAu, 0x00AA00u, 0x00AAAAu,
    0xAA0000u, 0xAA00AAu, 0xAA5500u, 0xAAAAAAu,
    0x555555u, 0x5555FFu, 0x55FF55u, 0x55FFFFu,
    0xFF5555u, 0xFF55FFu, 0xFFFF55u, 0xFFFFFFu
};

// Public-domain 8x8 bitmap font (font8x8_basic), ASCII 0x00-0x7F.
// Bit 0 (value 1) is the leftmost pixel of each row.
static const uint8_t font8x8_basic[128][8] = {
    {0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},
    {0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},
    {0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},
    {0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},
    {0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},
    {0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},
    {0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},
    {0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},{0,0,0,0,0,0,0,0},
    {0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}, // 0x20 ' '
    {0x18,0x3C,0x3C,0x18,0x18,0x00,0x18,0x00}, // !
    {0x36,0x36,0x00,0x00,0x00,0x00,0x00,0x00}, // "
    {0x36,0x36,0x7F,0x36,0x7F,0x36,0x36,0x00}, // #
    {0x0C,0x3E,0x03,0x1E,0x30,0x1F,0x0C,0x00}, // $
    {0x00,0x63,0x33,0x18,0x0C,0x66,0x63,0x00}, // %
    {0x1C,0x36,0x1C,0x6E,0x3B,0x33,0x6E,0x00}, // &
    {0x06,0x06,0x03,0x00,0x00,0x00,0x00,0x00}, // '
    {0x18,0x0C,0x06,0x06,0x06,0x0C,0x18,0x00}, // (
    {0x06,0x0C,0x18,0x18,0x18,0x0C,0x06,0x00}, // )
    {0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00}, // *
    {0x00,0x0C,0x0C,0x3F,0x0C,0x0C,0x00,0x00}, // +
    {0x00,0x00,0x00,0x00,0x00,0x0C,0x0C,0x06}, // ,
    {0x00,0x00,0x00,0x3F,0x00,0x00,0x00,0x00}, // -
    {0x00,0x00,0x00,0x00,0x00,0x0C,0x0C,0x00}, // .
    {0x60,0x30,0x18,0x0C,0x06,0x03,0x01,0x00}, // /
    {0x3E,0x63,0x73,0x7B,0x6F,0x67,0x3E,0x00}, // 0
    {0x0C,0x0E,0x0C,0x0C,0x0C,0x0C,0x3F,0x00}, // 1
    {0x1E,0x33,0x30,0x1C,0x06,0x33,0x3F,0x00}, // 2
    {0x1E,0x33,0x30,0x1C,0x30,0x33,0x1E,0x00}, // 3
    {0x38,0x3C,0x36,0x33,0x7F,0x30,0x78,0x00}, // 4
    {0x3F,0x03,0x1F,0x30,0x30,0x33,0x1E,0x00}, // 5
    {0x1C,0x06,0x03,0x1F,0x33,0x33,0x1E,0x00}, // 6
    {0x3F,0x33,0x30,0x18,0x0C,0x0C,0x0C,0x00}, // 7
    {0x1E,0x33,0x33,0x1E,0x33,0x33,0x1E,0x00}, // 8
    {0x1E,0x33,0x33,0x3E,0x30,0x18,0x0E,0x00}, // 9
    {0x00,0x0C,0x0C,0x00,0x00,0x0C,0x0C,0x00}, // :
    {0x00,0x0C,0x0C,0x00,0x00,0x0C,0x0C,0x06}, // ;
    {0x18,0x0C,0x06,0x03,0x06,0x0C,0x18,0x00}, // <
    {0x00,0x00,0x3F,0x00,0x00,0x3F,0x00,0x00}, // =
    {0x06,0x0C,0x18,0x30,0x18,0x0C,0x06,0x00}, // >
    {0x1E,0x33,0x30,0x18,0x0C,0x00,0x0C,0x00}, // ?
    {0x3E,0x63,0x7B,0x7B,0x7B,0x03,0x1E,0x00}, // @
    {0x0C,0x1E,0x33,0x33,0x3F,0x33,0x33,0x00}, // A
    {0x3F,0x66,0x66,0x3E,0x66,0x66,0x3F,0x00}, // B
    {0x3C,0x66,0x03,0x03,0x03,0x66,0x3C,0x00}, // C
    {0x1F,0x36,0x66,0x66,0x66,0x36,0x1F,0x00}, // D
    {0x7F,0x46,0x16,0x1E,0x16,0x46,0x7F,0x00}, // E
    {0x7F,0x46,0x16,0x1E,0x16,0x06,0x0F,0x00}, // F
    {0x3C,0x66,0x03,0x03,0x73,0x66,0x7C,0x00}, // G
    {0x33,0x33,0x33,0x3F,0x33,0x33,0x33,0x00}, // H
    {0x1E,0x0C,0x0C,0x0C,0x0C,0x0C,0x1E,0x00}, // I
    {0x78,0x30,0x30,0x30,0x33,0x33,0x1E,0x00}, // J
    {0x67,0x66,0x36,0x1E,0x36,0x66,0x67,0x00}, // K
    {0x0F,0x06,0x06,0x06,0x46,0x66,0x7F,0x00}, // L
    {0x63,0x77,0x7F,0x7F,0x6B,0x63,0x63,0x00}, // M
    {0x63,0x67,0x6F,0x7B,0x73,0x63,0x63,0x00}, // N
    {0x1C,0x36,0x63,0x63,0x63,0x36,0x1C,0x00}, // O
    {0x3F,0x66,0x66,0x3E,0x06,0x06,0x0F,0x00}, // P
    {0x1E,0x33,0x33,0x33,0x3B,0x1E,0x38,0x00}, // Q
    {0x3F,0x66,0x66,0x3E,0x36,0x66,0x67,0x00}, // R
    {0x1E,0x33,0x07,0x0E,0x38,0x33,0x1E,0x00}, // S
    {0x3F,0x2D,0x0C,0x0C,0x0C,0x0C,0x1E,0x00}, // T
    {0x33,0x33,0x33,0x33,0x33,0x33,0x3F,0x00}, // U
    {0x33,0x33,0x33,0x33,0x33,0x1E,0x0C,0x00}, // V
    {0x63,0x63,0x63,0x6B,0x7F,0x77,0x63,0x00}, // W
    {0x63,0x63,0x36,0x1C,0x1C,0x36,0x63,0x00}, // X
    {0x33,0x33,0x33,0x1E,0x0C,0x0C,0x1E,0x00}, // Y
    {0x7F,0x63,0x31,0x18,0x4C,0x66,0x7F,0x00}, // Z
    {0x1E,0x06,0x06,0x06,0x06,0x06,0x1E,0x00}, // [
    {0x03,0x06,0x0C,0x18,0x30,0x60,0x40,0x00}, // backslash
    {0x1E,0x18,0x18,0x18,0x18,0x18,0x1E,0x00}, // ]
    {0x08,0x1C,0x36,0x63,0x00,0x00,0x00,0x00}, // ^
    {0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xFF}, // _
    {0x0C,0x0C,0x18,0x00,0x00,0x00,0x00,0x00}, // `
    {0x00,0x00,0x1E,0x30,0x3E,0x33,0x6E,0x00}, // a
    {0x07,0x06,0x06,0x3E,0x66,0x66,0x3B,0x00}, // b
    {0x00,0x00,0x1E,0x33,0x03,0x33,0x1E,0x00}, // c
    {0x38,0x30,0x30,0x3E,0x33,0x33,0x6E,0x00}, // d
    {0x00,0x00,0x1E,0x33,0x3F,0x03,0x1E,0x00}, // e
    {0x1C,0x36,0x06,0x0F,0x06,0x06,0x0F,0x00}, // f
    {0x00,0x00,0x6E,0x33,0x33,0x3E,0x30,0x1F}, // g
    {0x07,0x06,0x36,0x6E,0x66,0x66,0x67,0x00}, // h
    {0x0C,0x00,0x0E,0x0C,0x0C,0x0C,0x1E,0x00}, // i
    {0x30,0x00,0x30,0x30,0x30,0x33,0x33,0x1E}, // j
    {0x07,0x06,0x66,0x36,0x1E,0x36,0x67,0x00}, // k
    {0x0E,0x0C,0x0C,0x0C,0x0C,0x0C,0x1E,0x00}, // l
    {0x00,0x00,0x33,0x7F,0x7F,0x6B,0x63,0x00}, // m
    {0x00,0x00,0x1F,0x33,0x33,0x33,0x33,0x00}, // n
    {0x00,0x00,0x1E,0x33,0x33,0x33,0x1E,0x00}, // o
    {0x00,0x00,0x3B,0x66,0x66,0x3E,0x06,0x0F}, // p
    {0x00,0x00,0x6E,0x33,0x33,0x3E,0x30,0x78}, // q
    {0x00,0x00,0x3B,0x6E,0x66,0x06,0x0F,0x00}, // r
    {0x00,0x00,0x3E,0x03,0x1E,0x30,0x1F,0x00}, // s
    {0x08,0x0C,0x3E,0x0C,0x0C,0x2C,0x18,0x00}, // t
    {0x00,0x00,0x33,0x33,0x33,0x33,0x6E,0x00}, // u
    {0x00,0x00,0x33,0x33,0x33,0x1E,0x0C,0x00}, // v
    {0x00,0x00,0x63,0x6B,0x7F,0x7F,0x36,0x00}, // w
    {0x00,0x00,0x63,0x36,0x1C,0x36,0x63,0x00}, // x
    {0x00,0x00,0x33,0x33,0x33,0x3E,0x30,0x1F}, // y
    {0x00,0x00,0x3F,0x19,0x0C,0x26,0x3F,0x00}, // z
    {0x38,0x0C,0x0C,0x07,0x0C,0x0C,0x38,0x00}, // {
    {0x18,0x18,0x18,0x00,0x18,0x18,0x18,0x00}, // |
    {0x07,0x0C,0x0C,0x38,0x0C,0x0C,0x07,0x00}, // }
    {0x6E,0x3B,0x00,0x00,0x00,0x00,0x00,0x00}, // ~
    {0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}  // 0x7F
};

static inline uint32_t fb_scale_chan(uint32_t v8, uint8_t bits) {
    if (bits >= 8) {
        return v8 << (bits - 8u);
    }
    return v8 >> (8u - bits);
}

static inline uint32_t fb_pack(uint32_t rgb) {
    uint32_t r = (rgb >> 16) & 0xFFu;
    uint32_t g = (rgb >> 8) & 0xFFu;
    uint32_t b = rgb & 0xFFu;
    r = fb_scale_chan(r, fb_rsize);
    g = fb_scale_chan(g, fb_gsize);
    b = fb_scale_chan(b, fb_bsize);
    return (r << fb_rpos) | (g << fb_gpos) | (b << fb_bpos);
}

static inline void fb_put_pixel(uint32_t x, uint32_t y, uint32_t pixel) {
    volatile uint8_t* p = fb_base + (uint64_t)y * fb_pitch + (uint64_t)x * fb_bytespp;
    for (uint8_t i = 0; i < fb_bytespp; i++) {
        p[i] = (uint8_t)(pixel >> (i * 8u));
    }
}

static uint32_t fb_cell_w(void) {
    return (uint32_t)FB_FONT_W * (uint32_t)fb_cell_scale;
}

static uint32_t fb_cell_h(void) {
    return (uint32_t)FB_FONT_H * (uint32_t)fb_cell_scale;
}

static void fb_draw_cell(unsigned int cx, unsigned int cy);
static void fb_draw_cursor(unsigned int cx, unsigned int cy);

static void console_fb_fill_text_panel(void) {
    if (!fb_active || !fb_base) {
        return;
    }
    uint32_t x0 = fb_origin_x;
    uint32_t y0 = fb_origin_y;
    uint32_t rw = VGA_WIDTH * fb_cell_w();
    uint32_t rh = VGA_HEIGHT * fb_cell_h();
    uint32_t pix = fb_pack(POPCORN_FB_BG_RGB);
    for (uint32_t y = y0; y < y0 + rh && y < fb_height; y++) {
        for (uint32_t x = x0; x < x0 + rw && x < fb_width; x++) {
            fb_put_pixel(x, y, pix);
        }
    }
}

static void console_fb_sync_cell(unsigned int cx, unsigned int cy) {
    if (!fb_active || cx >= VGA_WIDTH || cy >= VGA_HEIGHT) {
        return;
    }
    unsigned int pos = (cy * VGA_WIDTH + cx) * 2u;
    if (fb_text_rendered[pos] == vga_memory[pos] &&
        fb_text_rendered[pos + 1] == vga_memory[pos + 1]) {
        return;
    }
    fb_draw_cell(cx, cy);
    fb_text_rendered[pos] = vga_memory[pos];
    fb_text_rendered[pos + 1] = vga_memory[pos + 1];
}

static void console_fb_sync_cursor(void) {
    if (!fb_active) {
        return;
    }
    unsigned int cur = console_state.cursor_y * VGA_WIDTH + console_state.cursor_x;
    if (fb_prev_cursor != cur && fb_prev_cursor != 0xFFFFFFFFu) {
        console_fb_sync_cell(fb_prev_cursor % VGA_WIDTH, fb_prev_cursor / VGA_WIDTH);
    }
    fb_draw_cursor(console_state.cursor_x, console_state.cursor_y);
    fb_prev_cursor = cur;
}

static void fb_compute_layout(void) {
    uint32_t want_w = (fb_width * 9u) / 10u;
    uint32_t want_h = (fb_height * 9u) / 10u;
    uint32_t sw = want_w / (VGA_WIDTH * FB_FONT_W);
    uint32_t sh = want_h / (VGA_HEIGHT * FB_FONT_H);
    fb_cell_scale = (uint8_t)(sw < sh ? sw : sh);
    if (fb_cell_scale < 1u) {
        fb_cell_scale = 1u;
    }
    if (fb_cell_scale > 4u) {
        fb_cell_scale = 4u;
    }
    while (fb_cell_scale > 1u) {
        if (VGA_WIDTH * FB_FONT_W * (uint32_t)fb_cell_scale <= want_w &&
            VGA_HEIGHT * FB_FONT_H * (uint32_t)fb_cell_scale <= want_h) {
            break;
        }
        fb_cell_scale--;
    }
    uint32_t text_w = VGA_WIDTH * fb_cell_w();
    uint32_t text_h = VGA_HEIGHT * fb_cell_h();
    fb_origin_x = (fb_width - text_w) / 2u;
    fb_origin_y = (fb_height - text_h) / 2u;
}

// Render one text cell (char + attr) from the shadow buffer into the framebuffer.
static void fb_draw_cell(unsigned int cx, unsigned int cy) {
    unsigned int pos = (cy * VGA_WIDTH + cx) * 2;
    unsigned char ch = (unsigned char)vga_memory[pos];
    unsigned char attr = (unsigned char)vga_memory[pos + 1];
    uint32_t fgp = fb_pack(fb_palette[attr & 0x0Fu]);
    uint32_t bgp = fb_pack(fb_palette[(attr >> 4) & 0x07u]);
    const uint8_t* glyph = font8x8_basic[ch & 0x7Fu];
    uint32_t cw = fb_cell_w();
    uint32_t cell_h = fb_cell_h();
    uint32_t px0 = fb_origin_x + cx * cw;
    uint32_t py0 = fb_origin_y + cy * cell_h;
    for (uint32_t dy = 0; dy < cell_h; dy++) {
        for (uint32_t dx = 0; dx < cw; dx++) {
            fb_put_pixel(px0 + dx, py0 + dy, bgp);
        }
    }
    for (uint32_t row = 0; row < FB_FONT_H; row++) {
        uint8_t bits = glyph[row];
        for (uint32_t col = 0; col < FB_FONT_W; col++) {
            if ((bits & (1u << col)) == 0) {
                continue;
            }
            uint32_t gx = px0 + col * (uint32_t)fb_cell_scale;
            uint32_t gy = py0 + row * (uint32_t)fb_cell_scale;
            for (uint32_t sy = 0; sy < (uint32_t)fb_cell_scale; sy++) {
                for (uint32_t sx = 0; sx < (uint32_t)fb_cell_scale; sx++) {
                    fb_put_pixel(gx + sx, gy + sy, fgp);
                }
            }
        }
    }
}

static void fb_draw_cursor(unsigned int cx, unsigned int cy) {
    if (!console_state.cursor_visible) {
        return;
    }
    unsigned int pos = (cy * VGA_WIDTH + cx) * 2;
    unsigned char attr = (unsigned char)vga_memory[pos + 1];
    uint32_t fgp = fb_pack(fb_palette[attr & 0x0Fu]);
    uint32_t cw = fb_cell_w();
    uint32_t cell_h = fb_cell_h();
    uint32_t px0 = fb_origin_x + cx * cw;
    uint32_t py0 = fb_origin_y + cy * cell_h;
    for (uint32_t col = 0; col < cw; col++) {
        fb_put_pixel(px0 + col, py0 + cell_h - 1u, fgp);
        if (cell_h > 1u) {
            fb_put_pixel(px0 + col, py0 + cell_h - 2u, fgp);
        }
    }
}

bool console_fb_active(void) {
    return fb_active;
}

void console_fb_paint_background(uint32_t rgb) {
    if (!fb_active || !fb_base) {
        return;
    }
    fb_compute_layout();
    boot_fb_solid_fill(fb_base, fb_pitch, fb_width, fb_height, fb_bytespp, rgb);
}

void console_fb_relayout(void) {
    if (!fb_active) {
        return;
    }
    fb_compute_layout();
    console_fb_fill_text_panel();
    for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i++) {
        fb_text_rendered[i] = (char)0xFF;
    }
    fb_prev_cursor = 0xFFFFFFFFu;
}

char* console_get_buffer(void) {
    return vga_memory;
}

// Mirror the text shadow into the linear framebuffer (no-op in VGA text mode).
void console_present(void) {
    if (!fb_active) {
        return;
    }
    for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i += 2) {
        if (fb_text_rendered[i] != vga_memory[i] ||
            fb_text_rendered[i + 1] != vga_memory[i + 1]) {
            unsigned int cell = i / 2;
            fb_draw_cell(cell % VGA_WIDTH, cell / VGA_WIDTH);
            fb_text_rendered[i] = vga_memory[i];
            fb_text_rendered[i + 1] = vga_memory[i + 1];
        }
    }
    unsigned int cur = console_state.cursor_y * VGA_WIDTH + console_state.cursor_x;
    if (fb_prev_cursor != cur && fb_prev_cursor != 0xFFFFFFFFu) {
        fb_draw_cell(fb_prev_cursor % VGA_WIDTH, fb_prev_cursor / VGA_WIDTH);
    }
    fb_draw_cursor(console_state.cursor_x, console_state.cursor_y);
    fb_prev_cursor = cur;
}

// Set up framebuffer text backend from the Multiboot2 framebuffer tag.
// Falls back silently to VGA text mode if no usable framebuffer is available.
static void console_fb_init(void) {
    fb_active = false;
    const FramebufferInfo* fb = multiboot2_get_framebuffer();
    if (!fb || !fb->present) {
        return;
    }
    if (fb->type == MULTIBOOT_FRAMEBUFFER_TYPE_EGA_TEXT) {
        return;
    }
    if (fb->bpp < 15 || fb->bpp > 32) {
        return;
    }
    if (fb->width < 320 || fb->height < 200) {
        return;
    }

    uint32_t pitch = fb->pitch;
    if (pitch == 0) {
        pitch = fb->width * (uint32_t)((fb->bpp + 7) / 8);
    }
    /* UEFI boot path identity-maps the low 512 GiB (kernel.asm uefi_start, 1 GiB pages),
     * so accept any framebuffer below that ceiling. */
    uint64_t span = (uint64_t)pitch * fb->height;
    if (fb->addr == 0 || (fb->addr + span) > (512ULL << 30)) {
        return;
    }

    fb_base = (volatile uint8_t*)(uintptr_t)fb->addr;
    fb_width = fb->width;
    fb_height = fb->height;
    fb_pitch = pitch;
    fb_bytespp = (uint8_t)((fb->bpp + 7) / 8);
    fb_rpos = fb->red_pos;
    fb_rsize = fb->red_size ? fb->red_size : 8;
    fb_gpos = fb->green_pos;
    fb_gsize = fb->green_size ? fb->green_size : 8;
    fb_bpos = fb->blue_pos;
    fb_bsize = fb->blue_size ? fb->blue_size : 8;
    /* UEFI GOP / many GRUB builds use BGRx with zero mask fields in the tag. */
    if (fb->bpp >= 24 && fb->red_size == 0 && fb->green_size == 0 && fb->blue_size == 0) {
        fb_rpos = 16;
        fb_gpos = 8;
        fb_bpos = 0;
        fb_rsize = fb_gsize = fb_bsize = 8;
    }

    fb_active = true;
    vga_memory = fb_text_shadow;

    boot_identity_uncache_range(fb->addr, span);
    fb_compute_layout();

    /* Force full redraw on first present (no full-screen fill — very slow on 1920x1200 MMIO). */
    for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i++) {
        fb_text_rendered[i] = (char)0xFF;
    }
    fb_prev_cursor = 0xFFFFFFFFu;
}

// Double buffering support
static char back_buffer[VGA_MEMORY_SIZE];
static bool buffer_dirty = false;

// Scrollback buffer
static ScrollbackBuffer scrollback = {{0}, 0, 0};

// External variables from core/kernel.c
extern unsigned int current_loc;

// External port I/O functions
extern void write_port(unsigned short port, unsigned char data);
extern char read_port(unsigned short port);

// Update hardware VGA cursor position
static void update_hardware_cursor(unsigned int x, unsigned int y) {
    unsigned short position = (y * VGA_WIDTH) + x;
    
    // Cursor location low byte
    write_port(VGA_CTRL_PORT, VGA_CURSOR_LOC_LOW);
    write_port(VGA_DATA_PORT, (unsigned char)(position & 0xFF));
    
    // Cursor location high byte
    write_port(VGA_CTRL_PORT, VGA_CURSOR_LOC_HIGH);
    write_port(VGA_DATA_PORT, (unsigned char)((position >> 8) & 0xFF));
}

// Initialize the console system
void console_init(void) {
    console_state.cursor_x = 0;
    console_state.cursor_y = 0;
    console_state.current_color = CONSOLE_FG_COLOR;
    console_state.cursor_visible = true;
    console_state.double_buffer_enabled = false;
    console_state.scroll_offset = 0;
    
    // Initialize back buffer
    for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i += 2) {
        back_buffer[i] = ' ';
        back_buffer[i + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    
    // Initialize scrollback buffer
    for (unsigned int i = 0; i < SCROLLBACK_LINES * SCROLLBACK_LINE_SIZE; i++) {
        scrollback.buffer[i] = 0;
    }
    scrollback.current_line = 0;
    scrollback.total_lines = 0;

    /* UEFI/Ventoy GRUB2 often supplies VBE (tag 7) instead of framebuffer (tag 8). */
    multiboot2_parse_framebuffer();
    console_fb_init();

    console_clear();

    if (fb_active) {
        console_set_cursor(26, 11);
        console_println_color("POPCORN", 0x0A);
        console_present();
    }
}

// Clear the entire screen
void console_clear(void) {
    for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i += 2) {
        vga_memory[i] = ' ';
        vga_memory[i + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    console_state.cursor_x = 0;
    console_state.cursor_y = 0;
    current_loc = 0;

    update_hardware_cursor(0, 0);
    if (fb_active) {
        console_fb_fill_text_panel();
        for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i++) {
            fb_text_rendered[i] = vga_memory[i];
        }
        fb_prev_cursor = 0xFFFFFFFFu;
        return;
    }
    console_present();
}

// Set the current text color
void console_set_color(unsigned char color) {
    console_state.current_color = color;
}

// Set cursor position
void console_set_cursor(unsigned int x, unsigned int y) {
    if (x >= VGA_WIDTH) x = VGA_WIDTH - 1;
    if (y >= VGA_HEIGHT) y = VGA_HEIGHT - 1;
    
    console_state.cursor_x = x;
    console_state.cursor_y = y;
    current_loc = (y * VGA_WIDTH + x) * 2;
    
    update_hardware_cursor(x, y);
    if (fb_active) {
        console_fb_sync_cursor();
        return;
    }
    console_present();
}

// Put a single character at current cursor position
void console_putchar(char c) {
    if (c == '\n') {
        console_newline();
        return;
    }
    
    if (c == '\r') {
        console_state.cursor_x = 0;
        current_loc = console_state.cursor_y * VGA_WIDTH * 2;
        return;
    }
    
    if (c == '\b') {
        console_backspace();
        return;
    }
    
    unsigned int cx = console_state.cursor_x;
    unsigned int cy = console_state.cursor_y;
    unsigned int pos = (cy * VGA_WIDTH + cx) * 2;

    // Write character and color to appropriate buffer
    if (console_state.double_buffer_enabled) {
        back_buffer[pos] = c;
        back_buffer[pos + 1] = console_state.current_color;
        buffer_dirty = true;
    } else {
        vga_memory[pos] = c;
        vga_memory[pos + 1] = console_state.current_color;
    }
    
    // Move cursor
    console_state.cursor_x++;
    if (console_state.cursor_x >= VGA_WIDTH) {
        console_newline();
    }
    
    current_loc = (console_state.cursor_y * VGA_WIDTH + console_state.cursor_x) * 2;
    
    update_hardware_cursor(console_state.cursor_x, console_state.cursor_y);
    if (fb_active) {
        console_fb_sync_cell(cx, cy);
        console_fb_sync_cursor();
        return;
    }
    console_present();
}

// Print a string with current color
void console_print(const char* str) {
    while (*str) {
        console_putchar(*str++);
    }
}

// Print a string with specific color
void console_print_color(const char* str, unsigned char color) {
    unsigned char old_color = console_state.current_color;
    console_set_color(color);
    console_print(str);
    console_set_color(old_color);
}

// Print a string and add newline
void console_println(const char* str) {
    console_print(str);
    console_newline();
}

// Print a string with specific color and add newline
void console_println_color(const char* str, unsigned char color) {
    console_print_color(str, color);
    console_newline();
}

// Move to next line
void console_newline(void) {
    console_state.cursor_x = 0;
    console_state.cursor_y++;
    
    if (console_state.cursor_y >= VGA_HEIGHT) {
        console_scroll();
        console_state.cursor_y = VGA_HEIGHT - 1;
    }
    
    current_loc = console_state.cursor_y * VGA_WIDTH * 2;
    
    update_hardware_cursor(console_state.cursor_x, console_state.cursor_y);
    if (fb_active) {
        console_fb_sync_cursor();
        return;
    }
    console_present();
}

// Scroll the screen up by one line
void console_scroll(void) {
    // Save top line to scrollback before scrolling
    console_save_line(0);
    
    // Move all lines up by one
    for (unsigned int y = 0; y < VGA_HEIGHT - 1; y++) {
        for (unsigned int x = 0; x < VGA_WIDTH; x++) {
            unsigned int src_pos = ((y + 1) * VGA_WIDTH + x) * 2;
            unsigned int dst_pos = (y * VGA_WIDTH + x) * 2;
            
            vga_memory[dst_pos] = vga_memory[src_pos];
            vga_memory[dst_pos + 1] = vga_memory[src_pos + 1];
        }
    }
    
    // Clear the bottom line
    for (unsigned int x = 0; x < VGA_WIDTH; x++) {
        unsigned int pos = ((VGA_HEIGHT - 1) * VGA_WIDTH + x) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    
    // Reset scroll offset when new content appears
    console_state.scroll_offset = 0;
    console_present();
}

// Handle backspace
void console_backspace(void) {
    if (console_state.cursor_x > 0) {
        console_state.cursor_x--;
        unsigned int pos = (console_state.cursor_y * VGA_WIDTH + console_state.cursor_x) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
        current_loc = pos;
    } else if (console_state.cursor_y > 0) {
        // Move to end of previous line if at start of line
        console_state.cursor_y--;
        console_state.cursor_x = VGA_WIDTH - 1;
        unsigned int pos = (console_state.cursor_y * VGA_WIDTH + console_state.cursor_x) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
        current_loc = pos;
    }
    
    update_hardware_cursor(console_state.cursor_x, console_state.cursor_y);
    if (fb_active) {
        if (console_state.cursor_x > 0) {
            console_fb_sync_cell(console_state.cursor_x, console_state.cursor_y);
        } else if (console_state.cursor_y > 0) {
            console_fb_sync_cell(VGA_WIDTH - 1, console_state.cursor_y);
        }
        console_fb_sync_cursor();
        return;
    }
    console_present();
}

// Draw a box with borders
void console_draw_box(unsigned int x, unsigned int y, unsigned int width, unsigned int height, unsigned char color) {
    // Draw top and bottom borders
    for (unsigned int i = x; i < x + width; i++) {
        if (i < VGA_WIDTH) {
            // Top border
            if (y < VGA_HEIGHT) {
                unsigned int pos = (y * VGA_WIDTH + i) * 2;
                vga_memory[pos] = (i == x) ? '+' : (i == x + width - 1) ? '+' : '-';
                vga_memory[pos + 1] = color;
            }
            // Bottom border
            if (y + height - 1 < VGA_HEIGHT) {
                unsigned int pos = ((y + height - 1) * VGA_WIDTH + i) * 2;
                vga_memory[pos] = (i == x) ? '+' : (i == x + width - 1) ? '+' : '-';
                vga_memory[pos + 1] = color;
            }
        }
    }
    
    // Draw left and right borders
    for (unsigned int i = y; i < y + height; i++) {
        if (i < VGA_HEIGHT) {
            // Left border
            if (x < VGA_WIDTH) {
                unsigned int pos = (i * VGA_WIDTH + x) * 2;
                vga_memory[pos] = (i == y) ? '+' : (i == y + height - 1) ? '+' : '|';
                vga_memory[pos + 1] = color;
            }
            // Right border
            if (x + width - 1 < VGA_WIDTH) {
                unsigned int pos = (i * VGA_WIDTH + x + width - 1) * 2;
                vga_memory[pos] = (i == y) ? '+' : (i == y + height - 1) ? '+' : '|';
                vga_memory[pos + 1] = color;
            }
        }
    }
    console_present();
}

// Draw a header with title
void console_draw_header(const char* title) {
    console_set_cursor(0, 1);
    console_print_color("+------------------------------------------------------------------------------+", CONSOLE_HEADER_COLOR);
    console_newline();
    
    console_set_cursor(0, 2);
    console_print_color("|", CONSOLE_HEADER_COLOR);
    
    // Center the title
    unsigned int title_len = 0;
    while (title[title_len]) title_len++;
    unsigned int padding = (78 - title_len) / 2;
    
    for (unsigned int i = 0; i < padding; i++) {
        console_print_color(" ", CONSOLE_HEADER_COLOR);
    }
    console_print_color(title, CONSOLE_HEADER_COLOR);
    for (unsigned int i = 0; i < 78 - title_len - padding; i++) {
        console_print_color(" ", CONSOLE_HEADER_COLOR);
    }
    console_print_color("|", CONSOLE_HEADER_COLOR);
    console_newline();
    
    console_set_cursor(0, 3);
    console_print_color("+------------------------------------------------------------------------------+", CONSOLE_HEADER_COLOR);
    console_newline();
    console_newline();
}

// Draw command prompt
void console_draw_prompt(void) {
    console_print_color("popcorn@kernel:~$ ", CONSOLE_PROMPT_COLOR);
}

// Draw command prompt with current directory path
void console_draw_prompt_with_path(const char* path) {
    console_print_color("popcorn@kernel:", CONSOLE_PROMPT_COLOR);
    console_print_color(path, CONSOLE_INFO_COLOR);
    console_print_color("$ ", CONSOLE_PROMPT_COLOR);
}

static uint64_t console_read_tsc(void) {
    uint32_t lo;
    uint32_t hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    return ((uint64_t)hi << 32) | lo;
}

static void console_heartbeat_paint(uint64_t alive, uint64_t tsc_lo, uint64_t uefi_polls) {
    char num[24];
    char line[48];
    unsigned int i = 0;
    unsigned int j = 0;

    line[i++] = 'A';
    line[i++] = ':';
    uint64_to_str(alive, num);
    for (j = 0; num[j] && i < sizeof(line) - 20; j++) {
        line[i++] = num[j];
    }
    line[i++] = ' ';
    line[i++] = 'T';
    line[i++] = ':';
    uint64_to_str(tsc_lo & 0xFFFFFu, num);
    for (j = 0; num[j] && i < sizeof(line) - 12; j++) {
        line[i++] = num[j];
    }
    if (uefi_polls > 0) {
        line[i++] = ' ';
        line[i++] = 'u';
        line[i++] = ':';
        uint64_to_str(uefi_polls, num);
        for (j = 0; num[j] && i < sizeof(line) - 1; j++) {
            line[i++] = num[j];
        }
    }
    line[i] = '\0';

    for (j = 0; line[j] && (HEARTBEAT_COL + j) < VGA_WIDTH; j++) {
        unsigned int cx = HEARTBEAT_COL + j;
        unsigned int pos = (HEARTBEAT_ROW * VGA_WIDTH + cx) * 2u;
        vga_memory[pos] = line[j];
        vga_memory[pos + 1] = (char)(CONSOLE_BG_COLOR | CONSOLE_SUCCESS_COLOR);
        if (fb_active) {
            console_fb_sync_cell(cx, HEARTBEAT_ROW);
        }
    }
}

void console_heartbeat_tick(void) {
    console_alive_seq++;
    if ((console_alive_seq & 0x1Fu) != 0u) {
        return;
    }
    uint64_t polls = uefi_input_available() ? uefi_input_poll_attempts() : 0;
    uint64_t tsc = console_read_tsc();
    console_heartbeat_paint(console_alive_seq, tsc, polls);
}

// Print status bar at bottom
void console_print_status_bar(void) {
    // Save current cursor position
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;
    
    console_set_cursor(0, VGA_HEIGHT - 1);
    
    // Clear the line first
    for (unsigned int i = 0; i < VGA_WIDTH; i++) {
        unsigned int pos = ((VGA_HEIGHT - 1) * VGA_WIDTH + i) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    
    console_set_cursor(0, VGA_HEIGHT - 1);
    console_print_color("Status: Ready | help", CONSOLE_INFO_COLOR);

    if (fb_active) {
        for (unsigned int i = 0; i < VGA_WIDTH; i++) {
            console_fb_sync_cell(i, VGA_HEIGHT - 1u);
        }
    } else {
        console_present();
    }

    // Restore cursor position
    console_set_color(prev_color);
    console_set_cursor(prev_x, prev_y);
}

// Print error message
void console_print_error(const char* message) {
    console_print_color("ERROR: ", CONSOLE_ERROR_COLOR);
    console_print_color(message, CONSOLE_ERROR_COLOR);
    console_newline();
}

// Print success message
void console_print_success(const char* message) {
    console_print_color("SUCCESS: ", CONSOLE_SUCCESS_COLOR);
    console_print_color(message, CONSOLE_SUCCESS_COLOR);
    console_newline();
}

// Print info message
void console_print_info(const char* message) {
    console_print_color("INFO: ", CONSOLE_INFO_COLOR);
    console_print_color(message, CONSOLE_INFO_COLOR);
    console_newline();
}

// Print warning message
void console_print_warning(const char* message) {
    console_print_color("WARNING: ", CONSOLE_WARNING_COLOR);
    console_print_color(message, CONSOLE_WARNING_COLOR);
    console_newline();
}

// Make a color from foreground and background
unsigned char make_color(unsigned char foreground, unsigned char background) {
    return foreground | background;
}

// Center text on a line
void console_center_text(const char* text, unsigned int y, unsigned char color) {
    unsigned int text_len = 0;
    while (text[text_len]) text_len++;
    
    unsigned int x = (VGA_WIDTH - text_len) / 2;
    console_set_cursor(x, y);
    console_print_color(text, color);
}

// Draw a separator line
void console_draw_separator(unsigned int y, unsigned char color) {
    console_set_cursor(0, y);
    for (unsigned int i = 0; i < VGA_WIDTH; i++) {
        console_print_color("-", color);
    }
    console_newline();
}

// Enable or disable double buffering
void console_enable_double_buffer(bool enable) {
    console_state.double_buffer_enabled = enable;
    if (enable) {
        // Copy current VGA memory to back buffer
        for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i++) {
            back_buffer[i] = vga_memory[i];
        }
    } else {
        // Flush any pending changes
        console_flush();
    }
}

// Swap buffers (copy back buffer to VGA memory)
void console_swap_buffers(void) {
    if (!console_state.double_buffer_enabled) {
        return;
    }
    
    if (buffer_dirty) {
        for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i++) {
            vga_memory[i] = back_buffer[i];
        }
        buffer_dirty = false;
        console_present();
    }
}

// Flush back buffer to screen
void console_flush(void) {
    if (console_state.double_buffer_enabled) {
        console_swap_buffers();
    }
}

// Save current line to scrollback buffer
void console_save_line(unsigned int y) {
    if (y >= VGA_HEIGHT) return;
    
    unsigned int line_offset = (scrollback.current_line % SCROLLBACK_LINES) * SCROLLBACK_LINE_SIZE;
    unsigned int vga_offset = y * VGA_WIDTH * 2;
    
    // Copy line from VGA memory to scrollback
    for (unsigned int i = 0; i < SCROLLBACK_LINE_SIZE; i++) {
        scrollback.buffer[line_offset + i] = vga_memory[vga_offset + i];
    }
    
    scrollback.current_line++;
    if (scrollback.total_lines < SCROLLBACK_LINES) {
        scrollback.total_lines++;
    }
}

// Scroll up in history (Page Up)
void console_scroll_up(void) {
    if (console_state.scroll_offset >= (int)scrollback.total_lines - 1) {
        return;  // Already at top of history
    }
    
    console_state.scroll_offset++;
    console_restore_view();
}

// Scroll down in history (Page Down)
void console_scroll_down(void) {
    if (console_state.scroll_offset <= 0) {
        return;  // Already at current view
    }
    
    console_state.scroll_offset--;
    console_restore_view();
}

// Restore view based on scroll offset
void console_restore_view(void) {
    if (console_state.scroll_offset == 0) {
        return;
    }
    
    // Don't scroll beyond available history
    if (console_state.scroll_offset > (int)scrollback.total_lines) {
        console_state.scroll_offset = (int)scrollback.total_lines;
    }
    
    // Display history lines
    for (unsigned int y = 0; y < VGA_HEIGHT; y++) {
        // Calculate which history line to show at this screen position
        int line_index = (int)scrollback.current_line - console_state.scroll_offset + (int)y - (int)VGA_HEIGHT;
        
        if (line_index >= 0 && line_index < (int)scrollback.current_line) {
            // This line is in history, display it
            unsigned int buf_index = line_index % SCROLLBACK_LINES;
            unsigned int line_offset = buf_index * SCROLLBACK_LINE_SIZE;
            unsigned int vga_offset = y * VGA_WIDTH * 2;
            
            // Copy from scrollback to VGA
            for (unsigned int i = 0; i < SCROLLBACK_LINE_SIZE; i++) {
                vga_memory[vga_offset + i] = scrollback.buffer[line_offset + i];
            }
        } else {
            // Line not in history, clear it
            unsigned int vga_offset = y * VGA_WIDTH * 2;
            for (unsigned int i = 0; i < VGA_WIDTH; i++) {
                vga_memory[vga_offset + i * 2] = ' ';
                vga_memory[vga_offset + i * 2 + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
            }
        }
    }
    console_present();
}
