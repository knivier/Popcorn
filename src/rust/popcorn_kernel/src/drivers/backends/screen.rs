//! Display drive — full-bleed text grid (FB) or classic 80×25 VGA.

use super::{fb, vga};

const VGA_COLS: usize = 80;
const VGA_ROWS: usize = 25;
const MAX_COLS: usize = fb::MAX_COLS as usize;
const MAX_ROWS: usize = fb::MAX_ROWS as usize;
const CELL_BYTES: usize = MAX_COLS * MAX_ROWS * 2;
const VGA_MEM: usize = 0xB8000;

static mut CELLS: [u8; CELL_BYTES] = [0; CELL_BYTES];
static mut RENDERED: [u8; CELL_BYTES] = [0xFF; CELL_BYTES];
static mut DIRTY: [u8; MAX_ROWS] = [0; MAX_ROWS];
static mut DEFER: u32 = 0;
static mut PREV_CURSOR: u32 = 0xFFFF_FFFF;
static mut CURSOR_X: u32 = 0;
static mut CURSOR_Y: u32 = 0;
static mut CURSOR_VISIBLE: bool = true;
static mut MODE: i32 = 0; // 0=none 1=vga 2=fb
static mut COLS: usize = VGA_COLS;
static mut ROWS: usize = VGA_ROWS;

pub fn screen_backend() -> i32 {
    unsafe { MODE }
}

pub fn screen_cols() -> u32 {
    unsafe { COLS as u32 }
}

pub fn screen_rows() -> u32 {
    unsafe { ROWS as u32 }
}

fn cols() -> usize {
    unsafe { COLS }
}

fn rows() -> usize {
    unsafe { ROWS }
}

fn cell_off(x: usize, y: usize) -> usize {
    (y * cols() + x) * 2
}

pub fn cells_ptr() -> *mut u8 {
    if screen_backend() == 1 {
        VGA_MEM as *mut u8
    } else {
        unsafe { CELLS.as_mut_ptr() }
    }
}

pub fn init_vga() {
    unsafe {
        MODE = 1;
        COLS = VGA_COLS;
        ROWS = VGA_ROWS;
        PREV_CURSOR = 0xFFFF_FFFF;
        DEFER = 0;
        for d in DIRTY.iter_mut() {
            *d = 0;
        }
    }
    let _ = vga::probe();
}

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
        COLS = fb::cols() as usize;
        ROWS = fb::rows() as usize;
        PREV_CURSOR = 0xFFFF_FFFF;
        DEFER = 0;
        for d in DIRTY.iter_mut() {
            *d = 0;
        }
        for b in RENDERED.iter_mut() {
            *b = 0xFF;
        }
        let n = COLS * ROWS * 2;
        let mut i = 0;
        while i < n {
            CELLS[i] = b' ';
            CELLS[i + 1] = 0x07;
            i += 2;
        }
    }
    let _ = fb::probe();
    1
}

fn cell_at(x: u32, y: u32) -> (u8, u8) {
    let off = cell_off(x as usize, y as usize);
    unsafe { (CELLS[off], CELLS[off + 1]) }
}

fn write_cell_storage(x: u32, y: u32, ch: u8, attr: u8) {
    let off = cell_off(x as usize, y as usize);
    unsafe {
        CELLS[off] = ch;
        CELLS[off + 1] = attr;
    }
    if screen_backend() == 1 {
        vga::write_cell(x, y, ch, attr);
    }
}

pub fn screen_write_cell(x: u32, y: u32, ch: u8, attr: u8) {
    if (x as usize) >= cols() || (y as usize) >= rows() {
        return;
    }
    write_cell_storage(x, y, ch, attr);
    if screen_backend() == 2 {
        unsafe {
            if DEFER > 0 {
                DIRTY[y as usize] = 1;
            } else {
                sync_cell(x, y);
                sync_cursor();
            }
        }
    }
}

pub fn screen_set_cursor(x: u32, y: u32) {
    let x = if (x as usize) >= cols() {
        (cols() - 1) as u32
    } else {
        x
    };
    let y = if (y as usize) >= rows() {
        (rows() - 1) as u32
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
    if screen_backend() == 2 && (y as usize) < rows() {
        unsafe {
            DIRTY[y as usize] = 1;
        }
    }
}

pub fn screen_present() {
    if screen_backend() == 2 {
        for cy in 0..rows() as u32 {
            sync_row(cy);
        }
        sync_cursor();
        fb::flush();
    }
}

/// Push any shadow rows a paced flush left pending (call from idle loops).
pub fn screen_idle_flush() {
    if screen_backend() == 2 {
        fb::flush();
    }
}

pub fn screen_clear(attr: u8) {
    let mut y = 0u32;
    while y < rows() as u32 {
        let mut x = 0u32;
        while x < cols() as u32 {
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
        /* Mark every row dirty and let present/sync redraw cells. Do NOT
         * solid-fill the whole framebuffer here — that flashes blank frames
         * between boot stages and wipes readable startup text. */
        unsafe {
            for d in DIRTY.iter_mut().take(ROWS) {
                *d = 1;
            }
            for b in RENDERED.iter_mut().take(COLS * ROWS * 2) {
                *b = 0xFF;
            }
            PREV_CURSOR = 0xFFFF_FFFF;
        }
        flush_dirty();
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
            COLS = fb::cols() as usize;
            ROWS = fb::rows() as usize;
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

pub fn screen_invalidate() {
    if screen_backend() == 2 {
        unsafe {
            for b in RENDERED.iter_mut() {
                *b = 0xFF;
            }
            for d in DIRTY.iter_mut().take(ROWS) {
                *d = 1;
            }
        }
    }
}

fn sync_cell(cx: u32, cy: u32) {
    if screen_backend() != 2 {
        return;
    }
    let off = cell_off(cx as usize, cy as usize);
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
    for cx in 0..cols() as u32 {
        sync_cell(cx, cy);
    }
}

fn flush_dirty() {
    unsafe {
        for cy in 0..rows() {
            if DIRTY[cy] != 0 {
                sync_row(cy as u32);
                DIRTY[cy] = 0;
            }
        }
    }
    sync_cursor();
}

fn sync_cursor() {
    if screen_backend() != 2 {
        return;
    }
    unsafe {
        let cur = CURSOR_Y * cols() as u32 + CURSOR_X;
        /* Erase the old underline when the cursor moved *or* was just hidden. */
        if PREV_CURSOR != 0xFFFF_FFFF && (PREV_CURSOR != cur || !CURSOR_VISIBLE) {
            let ox = PREV_CURSOR % cols() as u32;
            let oy = PREV_CURSOR / cols() as u32;
            if oy < rows() as u32 {
                /* The underline was painted over the glyph while RENDERED still
                 * matches CELLS, so sync_cell would skip: repaint the cell directly. */
                let off = cell_off(ox as usize, oy as usize);
                let (ch, attr) = (CELLS[off], CELLS[off + 1]);
                fb::draw_cell(ox, oy, ch, attr);
                RENDERED[off] = ch;
                RENDERED[off + 1] = attr;
            }
            PREV_CURSOR = 0xFFFF_FFFF;
        }
        if CURSOR_VISIBLE {
            let (_ch, attr) = cell_at(CURSOR_X, CURSOR_Y);
            fb::draw_cursor(CURSOR_X, CURSOR_Y, attr, true);
            PREV_CURSOR = cur;
        }
    }
    /* Every update path ends here: push changed shadow rows to the fb
     * (paced; the idle loop and screen_present push whatever is left). */
    fb::flush_paced();
}

/// Allocate the framebuffer's write-back shadow (needs the kernel heap) and
/// re-render the screen into it. No-op on VGA text.
pub fn screen_enable_shadow() -> i32 {
    if screen_backend() != 2 || !fb::enable_shadow() {
        return 0;
    }
    unsafe {
        for b in RENDERED.iter_mut() {
            *b = 0xFF;
        }
        for d in DIRTY.iter_mut().take(ROWS) {
            *d = 1;
        }
        PREV_CURSOR = 0xFFFF_FFFF;
        if DEFER == 0 {
            flush_dirty();
        }
    }
    1
}

/// Scroll cell rows [0, n) up by one, blanking row n-1 with `attr`. Returns 1
/// when handled (framebuffer backend), 0 to let the caller scroll VGA text.
/// With the shadow the pixels move by memmove; without it only cells whose
/// content changed are redrawn (RENDERED still describes the screen).
pub fn screen_scroll_up(n: u32, attr: u8) -> i32 {
    if screen_backend() != 2 {
        return 0;
    }
    let n = if (n as usize) > rows() { rows() } else { n as usize };
    if n == 0 {
        return 1;
    }
    let line = cols() * 2;
    unsafe {
        if n >= 2 && fb::scroll_up(0, n as u32) {
            /* The cursor underline moves up with the pixels. Poison its
             * RENDERED entry before the shift below so the mark travels with
             * it (to row oy-1, or off the top) and that cell gets redrawn. */
            if PREV_CURSOR != 0xFFFF_FFFF {
                let ox = PREV_CURSOR % cols() as u32;
                let oy = PREV_CURSOR / cols() as u32;
                if (oy as usize) < n {
                    let off = cell_off(ox as usize, oy as usize);
                    RENDERED[off] = 0xFF;
                    RENDERED[off + 1] = 0xFF;
                    PREV_CURSOR = 0xFFFF_FFFF;
                }
            }
            /* RENDERED describes shadow pixels, which moved up with the memmove. */
            let r = RENDERED.as_mut_ptr();
            core::ptr::copy(r.add(line), r, (n - 1) * line);
            for b in RENDERED[(n - 1) * line..n * line].iter_mut() {
                *b = 0xFF;
            }
        }
        let c = CELLS.as_mut_ptr();
        core::ptr::copy(c.add(line), c, (n - 1) * line);
        let mut i = (n - 1) * line;
        while i < n * line {
            CELLS[i] = b' ';
            CELLS[i + 1] = attr;
            i += 2;
        }
        for d in DIRTY.iter_mut().take(n) {
            *d = 1;
        }
        if DEFER == 0 {
            flush_dirty();
        }
    }
    1
}
