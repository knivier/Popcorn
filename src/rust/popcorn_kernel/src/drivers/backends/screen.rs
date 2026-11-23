//! Display drive — full-bleed text grid (FB) or classic 80×25 VGA.
//!
//! Synchronisation: MODE/COLS/ROWS are independent scalars read on every
//! call (also lock-free from C), kept in relaxed atomics and only written
//! with the screen lock held. Everything updated together (cell grid, what
//! was rendered, dirty rows, defer depth, cursor) lives in [`ScreenState`]
//! behind one `IrqSpinLock`. Public entry points lock once and call inner
//! helpers that take `&mut ScreenState` (and `&mut fb::FbState` when they
//! draw). Lock order: screen state first, then `fb::lock()`.

use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering::Relaxed};

use super::{fb, vga};
use crate::sync::IrqSpinLock;

const VGA_COLS: usize = 80;
const VGA_ROWS: usize = 25;
const MAX_COLS: usize = fb::MAX_COLS as usize;
const MAX_ROWS: usize = fb::MAX_ROWS as usize;
const CELL_BYTES: usize = MAX_COLS * MAX_ROWS * 2;
const VGA_MEM: usize = 0xB8000;
const NO_CURSOR: u32 = 0xFFFF_FFFF;

static MODE: AtomicI32 = AtomicI32::new(0); // 0=none 1=vga 2=fb
static COLS: AtomicUsize = AtomicUsize::new(VGA_COLS);
static ROWS: AtomicUsize = AtomicUsize::new(VGA_ROWS);

struct ScreenState {
    /// Cell grid (char, attr). Its address is handed to C by `cells_ptr`;
    /// console.c writes through it directly from the console path (never
    /// while this lock is held: the holder runs with interrupts off).
    cells: [u8; CELL_BYTES],
    rendered: [u8; CELL_BYTES],
    dirty: [u8; MAX_ROWS],
    defer: u32,
    prev_cursor: u32,
    cursor_x: u32,
    cursor_y: u32,
    cursor_visible: bool,
}

static SCREEN: IrqSpinLock<ScreenState> = IrqSpinLock::new(ScreenState {
    cells: [0; CELL_BYTES],
    rendered: [0xFF; CELL_BYTES],
    dirty: [0; MAX_ROWS],
    defer: 0,
    prev_cursor: NO_CURSOR,
    cursor_x: 0,
    cursor_y: 0,
    cursor_visible: true,
});

pub fn screen_backend() -> i32 {
    MODE.load(Relaxed)
}

pub fn screen_cols() -> u32 {
    COLS.load(Relaxed) as u32
}

pub fn screen_rows() -> u32 {
    ROWS.load(Relaxed) as u32
}

fn cols() -> usize {
    COLS.load(Relaxed)
}

fn rows() -> usize {
    ROWS.load(Relaxed)
}

fn cell_off(x: usize, y: usize) -> usize {
    (y * cols() + x) * 2
}

pub fn cells_ptr() -> *mut u8 {
    if screen_backend() == 1 {
        VGA_MEM as *mut u8
    } else {
        /* The array lives inside a static, so the address is stable after the
         * guard is dropped. */
        SCREEN.lock().cells.as_mut_ptr()
    }
}

pub fn init_vga() {
    {
        let mut s = SCREEN.lock();
        MODE.store(1, Relaxed);
        COLS.store(VGA_COLS, Relaxed);
        ROWS.store(VGA_ROWS, Relaxed);
        s.prev_cursor = NO_CURSOR;
        s.defer = 0;
        for d in s.dirty.iter_mut() {
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
    {
        let mut s = SCREEN.lock();
        MODE.store(2, Relaxed);
        let (c, r) = (fb::cols() as usize, fb::rows() as usize);
        COLS.store(c, Relaxed);
        ROWS.store(r, Relaxed);
        s.prev_cursor = NO_CURSOR;
        s.defer = 0;
        for d in s.dirty.iter_mut() {
            *d = 0;
        }
        for b in s.rendered.iter_mut() {
            *b = 0xFF;
        }
        let n = c * r * 2;
        let mut i = 0;
        while i < n {
            s.cells[i] = b' ';
            s.cells[i + 1] = 0x07;
            i += 2;
        }
    }
    let _ = fb::probe();
    1
}

fn cell_at(s: &ScreenState, x: u32, y: u32) -> (u8, u8) {
    let off = cell_off(x as usize, y as usize);
    (s.cells[off], s.cells[off + 1])
}

fn write_cell_storage(s: &mut ScreenState, x: u32, y: u32, ch: u8, attr: u8) {
    let off = cell_off(x as usize, y as usize);
    s.cells[off] = ch;
    s.cells[off + 1] = attr;
    if screen_backend() == 1 {
        vga::write_cell(x, y, ch, attr);
    }
}

pub fn screen_write_cell(x: u32, y: u32, ch: u8, attr: u8) {
    if (x as usize) >= cols() || (y as usize) >= rows() {
        return;
    }
    let mut s = SCREEN.lock();
    write_cell_storage(&mut s, x, y, ch, attr);
    if screen_backend() == 2 {
        if s.defer > 0 {
            s.dirty[y as usize] = 1;
        } else {
            let mut f = fb::lock();
            sync_cell(&mut s, &mut f, x, y);
            sync_cursor(&mut s, &mut f);
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
    let mut s = SCREEN.lock();
    s.cursor_x = x;
    s.cursor_y = y;
    if screen_backend() == 1 {
        vga::set_cursor(x, y);
    } else if screen_backend() == 2 {
        sync_cursor(&mut s, &mut fb::lock());
    }
}

pub fn screen_set_cursor_visible(visible: i32) {
    let mut s = SCREEN.lock();
    s.cursor_visible = visible != 0;
    if screen_backend() == 2 {
        sync_cursor(&mut s, &mut fb::lock());
    }
}

pub fn screen_sync_begin() {
    if screen_backend() == 2 {
        SCREEN.lock().defer += 1;
    }
}

pub fn screen_sync_end() {
    if screen_backend() != 2 {
        return;
    }
    let mut s = SCREEN.lock();
    if s.defer > 0 {
        s.defer -= 1;
    }
    if s.defer == 0 {
        flush_dirty(&mut s, &mut fb::lock());
    }
}

pub fn screen_mark_row(y: u32) {
    if screen_backend() == 2 && (y as usize) < rows() {
        SCREEN.lock().dirty[y as usize] = 1;
    }
}

pub fn screen_present() {
    if screen_backend() == 2 {
        let mut s = SCREEN.lock();
        let mut f = fb::lock();
        for cy in 0..rows() as u32 {
            sync_row(&mut s, &mut f, cy);
        }
        sync_cursor(&mut s, &mut f);
        f.flush();
    }
}

/// Push any shadow rows a paced flush left pending (call from idle loops).
pub fn screen_idle_flush() {
    if screen_backend() == 2 {
        fb::lock().flush();
    }
}

pub fn screen_clear(attr: u8) {
    let mut s = SCREEN.lock();
    let mut y = 0u32;
    while y < rows() as u32 {
        let mut x = 0u32;
        while x < cols() as u32 {
            write_cell_storage(&mut s, x, y, b' ', attr);
            x += 1;
        }
        y += 1;
    }
    s.cursor_x = 0;
    s.cursor_y = 0;
    if screen_backend() == 1 {
        vga::set_cursor(0, 0);
    } else if screen_backend() == 2 {
        /* Mark every row dirty and let present/sync redraw cells. Do NOT
         * solid-fill the whole framebuffer here — that flashes blank frames
         * between boot stages and wipes readable startup text. */
        let (c, r) = (cols(), rows());
        for d in s.dirty.iter_mut().take(r) {
            *d = 1;
        }
        for b in s.rendered.iter_mut().take(c * r * 2) {
            *b = 0xFF;
        }
        s.prev_cursor = NO_CURSOR;
        flush_dirty(&mut s, &mut fb::lock());
    }
}

pub fn screen_paint_bg(rgb: u32) {
    if screen_backend() == 2 {
        fb::lock().paint_background(rgb);
    }
}

pub fn screen_relayout() {
    if screen_backend() == 2 {
        let mut s = SCREEN.lock();
        fb::lock().relayout();
        COLS.store(fb::cols() as usize, Relaxed);
        ROWS.store(fb::rows() as usize, Relaxed);
        for b in s.rendered.iter_mut() {
            *b = 0xFF;
        }
        s.prev_cursor = NO_CURSOR;
    }
}

pub fn screen_fill_panel() {
    if screen_backend() == 2 {
        fb::lock().fill_text_panel();
    }
}

pub fn screen_invalidate() {
    if screen_backend() == 2 {
        let mut s = SCREEN.lock();
        let r = rows();
        for b in s.rendered.iter_mut() {
            *b = 0xFF;
        }
        for d in s.dirty.iter_mut().take(r) {
            *d = 1;
        }
    }
}

fn sync_cell(s: &mut ScreenState, f: &mut fb::FbState, cx: u32, cy: u32) {
    if screen_backend() != 2 {
        return;
    }
    let off = cell_off(cx as usize, cy as usize);
    if s.rendered[off] == s.cells[off] && s.rendered[off + 1] == s.cells[off + 1] {
        return;
    }
    let ch = s.cells[off];
    let attr = s.cells[off + 1];
    f.draw_cell(cx, cy, ch, attr);
    s.rendered[off] = ch;
    s.rendered[off + 1] = attr;
}

fn sync_row(s: &mut ScreenState, f: &mut fb::FbState, cy: u32) {
    for cx in 0..cols() as u32 {
        sync_cell(s, f, cx, cy);
    }
}

fn flush_dirty(s: &mut ScreenState, f: &mut fb::FbState) {
    for cy in 0..rows() {
        if s.dirty[cy] != 0 {
            sync_row(s, f, cy as u32);
            s.dirty[cy] = 0;
        }
    }
    sync_cursor(s, f);
}

fn sync_cursor(s: &mut ScreenState, f: &mut fb::FbState) {
    if screen_backend() != 2 {
        return;
    }
    let c = cols() as u32;
    let cur = s.cursor_y * c + s.cursor_x;
    /* Erase the old underline when the cursor moved *or* was just hidden. */
    if s.prev_cursor != NO_CURSOR && (s.prev_cursor != cur || !s.cursor_visible) {
        let ox = s.prev_cursor % c;
        let oy = s.prev_cursor / c;
        if oy < rows() as u32 {
            /* The underline was painted over the glyph while RENDERED still
             * matches CELLS, so sync_cell would skip: repaint the cell directly. */
            let off = cell_off(ox as usize, oy as usize);
            let (ch, attr) = (s.cells[off], s.cells[off + 1]);
            f.draw_cell(ox, oy, ch, attr);
            s.rendered[off] = ch;
            s.rendered[off + 1] = attr;
        }
        s.prev_cursor = NO_CURSOR;
    }
    if s.cursor_visible {
        let (_ch, attr) = cell_at(s, s.cursor_x, s.cursor_y);
        f.draw_cursor(s.cursor_x, s.cursor_y, attr, true);
        s.prev_cursor = cur;
    }
    /* Every update path ends here: push changed shadow rows to the fb
     * (paced; the idle loop and screen_present push whatever is left). */
    f.flush_paced();
}

/// Allocate the framebuffer's write-back shadow (needs the kernel heap) and
/// re-render the screen into it. No-op on VGA text.
pub fn screen_enable_shadow() -> i32 {
    /* fb::enable_shadow allocates: call it before taking any display lock. */
    if screen_backend() != 2 || !fb::enable_shadow() {
        return 0;
    }
    let mut s = SCREEN.lock();
    let r = rows();
    for b in s.rendered.iter_mut() {
        *b = 0xFF;
    }
    for d in s.dirty.iter_mut().take(r) {
        *d = 1;
    }
    s.prev_cursor = NO_CURSOR;
    if s.defer == 0 {
        flush_dirty(&mut s, &mut fb::lock());
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
    let mut s = SCREEN.lock();
    let mut f = fb::lock();
    if n >= 2 && f.scroll_up(0, n as u32) {
        /* The cursor underline moves up with the pixels. Poison its
         * RENDERED entry before the shift below so the mark travels with
         * it (to row oy-1, or off the top) and that cell gets redrawn. */
        if s.prev_cursor != NO_CURSOR {
            let ox = s.prev_cursor % cols() as u32;
            let oy = s.prev_cursor / cols() as u32;
            if (oy as usize) < n {
                let off = cell_off(ox as usize, oy as usize);
                s.rendered[off] = 0xFF;
                s.rendered[off + 1] = 0xFF;
                s.prev_cursor = NO_CURSOR;
            }
        }
        /* RENDERED describes shadow pixels, which moved up with the memmove. */
        s.rendered.copy_within(line..n * line, 0);
        for b in s.rendered[(n - 1) * line..n * line].iter_mut() {
            *b = 0xFF;
        }
    }
    s.cells.copy_within(line..n * line, 0);
    let mut i = (n - 1) * line;
    while i < n * line {
        s.cells[i] = b' ';
        s.cells[i + 1] = attr;
        i += 2;
    }
    for d in s.dirty.iter_mut().take(n) {
        *d = 1;
    }
    if s.defer == 0 {
        flush_dirty(&mut s, &mut f);
    }
    1
}
