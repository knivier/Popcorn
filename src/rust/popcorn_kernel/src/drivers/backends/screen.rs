//! Display drive — console/shell call here to write cells / present.

use super::{fb, vga};

const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;
const CELL_BYTES: usize = VGA_WIDTH * VGA_HEIGHT * 2;
const VGA_MEM: usize = 0xB8000;

static mut CELLS: [u8; CELL_BYTES] = [0; CELL_BYTES];
static mut RENDERED: [u8; CELL_BYTES] = [0xFF; CELL_BYTES];
static mut DIRTY_ROWS: u32 = 0;
static mut DEFER: u32 = 0;
static mut PREV_CURSOR: u32 = 0xFFFF_FFFF;
static mut CURSOR_X: u32 = 0;
static mut CURSOR_Y: u32 = 0;
static mut CURSOR_VISIBLE: bool = true;
static mut MODE: i32 = 0; // 0=none 1=vga 2=fb

pub fn screen_backend() -> i32 {
    unsafe { MODE }
}

pub fn cells_ptr() -> *mut u8 {
    if screen_backend() == 1 {
        VGA_MEM as *mut u8
    } else {
        unsafe { CELLS.as_mut_ptr() }
    }
}

/// Start classic VGA text (0xB8000). Called when no GOP handoff.
pub fn init_vga() {
    unsafe {
        MODE = 1;
        PREV_CURSOR = 0xFFFF_FFFF;
        DIRTY_ROWS = 0;
        DEFER = 0;
    }
    let _ = vga::probe();
}

/// Start GOP framebuffer text. `ty` is Multiboot framebuffer type (2 = EGA text → reject).
pub fn init_fb(
    addr: u64,
    pitch: u32,
    width: u32,
    height: u32,
    bpp: u8,
    ty: u8,
    red_pos: u8,
    red_size: u8,
    green_pos: u8,
    green_size: u8,
    blue_pos: u8,
    blue_size: u8,
) -> i32 {
    if ty == 2 {
        init_vga();
        return 0;
    }
    let ok = fb::init_from_handoff(&fb::FbHandoff {
        addr,
        pitch,
        width,
        height,
        bpp,
        red_pos,
        red_size,
        green_pos,
        green_size,
        blue_pos,
        blue_size,
    });
    if !ok {
        init_vga();
        return 0;
    }
    unsafe {
        MODE = 2;
        PREV_CURSOR = 0xFFFF_FFFF;
        DIRTY_ROWS = 0;
        DEFER = 0;
        for b in RENDERED.iter_mut() {
            *b = 0xFF;
        }
        // Seed cell buffer with blanks.
        let mut i = 0;
        while i < CELL_BYTES {
            CELLS[i] = b' ';
            CELLS[i + 1] = 0x07;
            i += 2;
        }
    }
    let _ = fb::probe();
    1
}

fn cell_at(x: u32, y: u32) -> (u8, u8) {
    let off = (y as usize * VGA_WIDTH + x as usize) * 2;
    unsafe { (CELLS[off], CELLS[off + 1]) }
}

fn write_cell_storage(x: u32, y: u32, ch: u8, attr: u8) {
    let off = (y as usize * VGA_WIDTH + x as usize) * 2;
    unsafe {
        CELLS[off] = ch;
        CELLS[off + 1] = attr;
    }
    if screen_backend() == 1 {
        vga::write_cell(x, y, ch, attr);
    }
}

pub fn screen_write_cell(x: u32, y: u32, ch: u8, attr: u8) {
    if x >= VGA_WIDTH as u32 || y >= VGA_HEIGHT as u32 {
        return;
    }
    write_cell_storage(x, y, ch, attr);
    if screen_backend() == 2 {
        unsafe {
            if DEFER > 0 {
                DIRTY_ROWS |= 1 << y;
            } else {
                sync_cell(x, y);
                sync_cursor();
            }
        }
    }
}

pub fn screen_set_cursor(x: u32, y: u32) {
    let x = if x >= VGA_WIDTH as u32 {
        (VGA_WIDTH - 1) as u32
    } else {
        x
    };
    let y = if y >= VGA_HEIGHT as u32 {
        (VGA_HEIGHT - 1) as u32
    } else {
        y
    };
    unsafe {
        CURSOR_X = x;
        CURSOR_Y = y;
    }
    if screen_backend() == 1 {
        vga::set_cursor(x, y);
    } else if screen_backend() == 2 {
        sync_cursor();
    }
}

pub fn screen_set_cursor_visible(visible: i32) {
    unsafe {
        CURSOR_VISIBLE = visible != 0;
    }
    if screen_backend() == 2 {
        sync_cursor();
    }
}

pub fn screen_sync_begin() {
    if screen_backend() == 2 {
        unsafe {
            DEFER += 1;
        }
    }
}

pub fn screen_sync_end() {
    if screen_backend() != 2 {
        return;
    }
    unsafe {
        if DEFER > 0 {
            DEFER -= 1;
        }
        if DEFER == 0 {
            flush_dirty();
        }
    }
}

pub fn screen_mark_row(y: u32) {
    if screen_backend() == 2 && y < VGA_HEIGHT as u32 {
        unsafe {
            DIRTY_ROWS |= 1 << y;
        }
    }
}

pub fn screen_present() {
    if screen_backend() == 2 {
        for cy in 0..VGA_HEIGHT as u32 {
            sync_row(cy);
        }
        sync_cursor();
    }
}

pub fn screen_clear(attr: u8) {
    let mut y = 0u32;
    while y < VGA_HEIGHT as u32 {
        let mut x = 0u32;
        while x < VGA_WIDTH as u32 {
            write_cell_storage(x, y, b' ', attr);
            x += 1;
        }
        y += 1;
    }
    unsafe {
        CURSOR_X = 0;
        CURSOR_Y = 0;
    }
    if screen_backend() == 1 {
        vga::set_cursor(0, 0);
    } else if screen_backend() == 2 {
        fb::fill_text_panel();
        unsafe {
            for i in 0..CELL_BYTES {
                RENDERED[i] = CELLS[i];
            }
            PREV_CURSOR = 0xFFFF_FFFF;
        }
    }
}

pub fn screen_paint_bg(rgb: u32) {
    if screen_backend() == 2 {
        fb::paint_background(rgb);
    }
}

pub fn screen_relayout() {
    if screen_backend() == 2 {
        fb::relayout();
        unsafe {
            for b in RENDERED.iter_mut() {
                *b = 0xFF;
            }
            PREV_CURSOR = 0xFFFF_FFFF;
        }
    }
}

pub fn screen_fill_panel() {
    if screen_backend() == 2 {
        fb::fill_text_panel();
    }
}

/// After C mutates the cell buffer in place (scroll/pops), force redraw.
pub fn screen_invalidate() {
    if screen_backend() == 2 {
        unsafe {
            for b in RENDERED.iter_mut() {
                *b = 0xFF;
            }
            DIRTY_ROWS = (1 << VGA_HEIGHT) - 1;
        }
    }
}

fn sync_cell(cx: u32, cy: u32) {
    if screen_backend() != 2 {
        return;
    }
    let off = (cy as usize * VGA_WIDTH + cx as usize) * 2;
    unsafe {
        if RENDERED[off] == CELLS[off] && RENDERED[off + 1] == CELLS[off + 1] {
            return;
        }
        let ch = CELLS[off];
        let attr = CELLS[off + 1];
        fb::draw_cell(cx, cy, ch, attr);
        RENDERED[off] = ch;
        RENDERED[off + 1] = attr;
    }
}

fn sync_row(cy: u32) {
    for cx in 0..VGA_WIDTH as u32 {
        sync_cell(cx, cy);
    }
}

fn flush_dirty() {
    unsafe {
        for cy in 0..VGA_HEIGHT as u32 {
            if DIRTY_ROWS & (1 << cy) != 0 {
                sync_row(cy);
            }
        }
        DIRTY_ROWS = 0;
    }
    sync_cursor();
}

fn sync_cursor() {
    if screen_backend() != 2 {
        return;
    }
    unsafe {
        let cur = CURSOR_Y * VGA_WIDTH as u32 + CURSOR_X;
        if PREV_CURSOR != cur && PREV_CURSOR != 0xFFFF_FFFF {
            let ox = PREV_CURSOR % VGA_WIDTH as u32;
            let oy = PREV_CURSOR / VGA_WIDTH as u32;
            sync_cell(ox, oy);
        }
        let (_ch, attr) = cell_at(CURSOR_X, CURSOR_Y);
        fb::draw_cursor(CURSOR_X, CURSOR_Y, attr, CURSOR_VISIBLE);
        PREV_CURSOR = cur;
    }
}
