//! Framebuffer /dev/fb0 — full-bleed GOP text at native 8×16 glyphs.

use super::font8x16::FONT8X16;

pub const FONT_W: u32 = 8;
pub const FONT_H: u32 = 16;
pub const MAX_COLS: u32 = 240;
pub const MAX_ROWS: u32 = 70;
const POPCORN_FB_BG_RGB: u32 = 0x001018;

static mut ACTIVE: bool = false;
static mut BASE: usize = 0;
static mut PITCH: u32 = 0;
static mut WIDTH: u32 = 0;
static mut HEIGHT: u32 = 0;
static mut BYTESPP: u8 = 0;
static mut RPOS: u8 = 0;
static mut RSIZE: u8 = 8;
static mut GPOS: u8 = 0;
static mut GSIZE: u8 = 8;
static mut BPOS: u8 = 0;
static mut BSIZE: u8 = 8;
static mut COLS: u32 = 80;
static mut ROWS: u32 = 25;
static mut ORIGIN_X: u32 = 0;
static mut ORIGIN_Y: u32 = 0;
static mut SCALE: u8 = 1;

extern "C" {
    fn boot_fb_solid_fill(
        base: *mut u8,
        pitch: u32,
        width: u32,
        height: u32,
        bytespp: u8,
        rgb: u32,
    );
    fn boot_identity_uncache_range(phys: u64, len: u64);
}

static PALETTE: [u32; 16] = [
    POPCORN_FB_BG_RGB,
    0x0000AA,
    0x00AA00,
    0x00AAAA,
    0xAA0000,
    0xAA00AA,
    0xAA5500,
    0xAAAAAA,
    0x555555,
    0x5555FF,
    0x55FF55,
    0x55FFFF,
    0xFF5555,
    0xFF55FF,
    0xFFFF55,
    0xFFFFFF,
];

pub fn probe() -> Result<(), &'static str> {
    if !is_active() {
        return Err("fb not handed off");
    }
    Ok(())
}

pub fn is_active() -> bool {
    unsafe { ACTIVE }
}

pub fn cols() -> u32 {
    unsafe { COLS }
}

pub fn rows() -> u32 {
    unsafe { ROWS }
}

pub struct FbHandoff {
    pub addr: u64,
    pub pitch: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u8,
    pub red_pos: u8,
    pub red_size: u8,
    pub green_pos: u8,
    pub green_size: u8,
    pub blue_pos: u8,
    pub blue_size: u8,
}

pub fn init_from_handoff(h: &FbHandoff) -> bool {
    if h.addr == 0 || h.bpp < 15 || h.bpp > 32 || h.width < 320 || h.height < 200 {
        return false;
    }
    let mut pitch = h.pitch;
    if pitch == 0 {
        pitch = h.width * (((h.bpp as u32) + 7) / 8);
    }
    let bytespp = ((h.bpp as u32 + 7) / 8) as u8;
    let span = pitch as u64 * h.height as u64;
    if h.addr + span > (512u64 << 30) {
        return false;
    }

    let mut rpos = h.red_pos;
    let mut rsize = if h.red_size != 0 { h.red_size } else { 8 };
    let mut gpos = h.green_pos;
    let mut gsize = if h.green_size != 0 { h.green_size } else { 8 };
    let mut bpos = h.blue_pos;
    let mut bsize = if h.blue_size != 0 { h.blue_size } else { 8 };
    if h.bpp >= 24 && h.red_size == 0 && h.green_size == 0 && h.blue_size == 0 {
        rpos = 16;
        gpos = 8;
        bpos = 0;
        rsize = 8;
        gsize = 8;
        bsize = 8;
    }

    unsafe {
        boot_identity_uncache_range(h.addr, span);
        BASE = h.addr as usize;
        PITCH = pitch;
        WIDTH = h.width;
        HEIGHT = h.height;
        BYTESPP = bytespp;
        RPOS = rpos;
        RSIZE = rsize;
        GPOS = gpos;
        GSIZE = gsize;
        BPOS = bpos;
        BSIZE = bsize;
        ACTIVE = true;
    }
    compute_layout();
    true
}

fn cell_w() -> u32 {
    FONT_W * unsafe { SCALE as u32 }
}

fn cell_h() -> u32 {
    FONT_H * unsafe { SCALE as u32 }
}

fn compute_layout() {
    unsafe {
        /* Native 8×16 → ~160×45 on 720p / ~240×67 on 1080p. */
        SCALE = 1;
        let cw = FONT_W;
        let ch = FONT_H;
        let mut c = WIDTH / cw;
        let mut r = HEIGHT / ch;
        if c < 40 {
            c = 40;
        }
        if r < 15 {
            r = 15;
        }
        if c > MAX_COLS {
            c = MAX_COLS;
        }
        if r > MAX_ROWS {
            r = MAX_ROWS;
        }
        COLS = c;
        ROWS = r;
        let text_w = c * cw;
        let text_h = r * ch;
        ORIGIN_X = if WIDTH > text_w { (WIDTH - text_w) / 2 } else { 0 };
        ORIGIN_Y = if HEIGHT > text_h { (HEIGHT - text_h) / 2 } else { 0 };
    }
}

fn scale_chan(v8: u32, bits: u8) -> u32 {
    if bits >= 8 {
        v8 << (bits - 8)
    } else {
        v8 >> (8 - bits)
    }
}

fn pack(rgb: u32) -> u32 {
    unsafe {
        let r = scale_chan((rgb >> 16) & 0xFF, RSIZE);
        let g = scale_chan((rgb >> 8) & 0xFF, GSIZE);
        let b = scale_chan(rgb & 0xFF, BSIZE);
        (r << RPOS) | (g << GPOS) | (b << BPOS)
    }
}

fn put_pixel(x: u32, y: u32, pixel: u32) {
    unsafe {
        if BASE == 0 || x >= WIDTH || y >= HEIGHT {
            return;
        }
        let p = (BASE as *mut u8)
            .add((y as usize) * (PITCH as usize) + (x as usize) * (BYTESPP as usize));
        for i in 0..BYTESPP as usize {
            *p.add(i) = (pixel >> (i * 8)) as u8;
        }
    }
}

pub fn paint_background(rgb: u32) {
    if !is_active() {
        return;
    }
    unsafe {
        boot_fb_solid_fill(BASE as *mut u8, PITCH, WIDTH, HEIGHT, BYTESPP, rgb);
    }
}

pub fn fill_text_panel() {
    if !is_active() {
        return;
    }
    paint_background(POPCORN_FB_BG_RGB);
    let pix = pack(POPCORN_FB_BG_RGB);
    let cw = cell_w();
    let ch = cell_h();
    unsafe {
        let x0 = ORIGIN_X;
        let y0 = ORIGIN_Y;
        let rw = COLS * cw;
        let rh = ROWS * ch;
        let mut y = y0;
        while y < y0 + rh && y < HEIGHT {
            let mut x = x0;
            while x < x0 + rw && x < WIDTH {
                put_pixel(x, y, pix);
                x += 1;
            }
            y += 1;
        }
    }
}

pub fn relayout() {
    if !is_active() {
        return;
    }
    compute_layout();
    fill_text_panel();
}

pub fn draw_cell(cx: u32, cy: u32, ch: u8, attr: u8) {
    if !is_active() || cx >= cols() || cy >= rows() {
        return;
    }
    let fgp = pack(PALETTE[(attr & 0x0F) as usize]);
    let bgp = pack(PALETTE[((attr >> 4) & 0x07) as usize]);
    let glyph = &FONT8X16[(ch & 0x7F) as usize];
    let cw = cell_w();
    let chh = cell_h();
    let scale = unsafe { SCALE as u32 };
    let px0 = unsafe { ORIGIN_X } + cx * cw;
    let py0 = unsafe { ORIGIN_Y } + cy * chh;
    for dy in 0..chh {
        for dx in 0..cw {
            put_pixel(px0 + dx, py0 + dy, bgp);
        }
    }
    for row in 0..FONT_H {
        let bits = glyph[row as usize];
        for col in 0..FONT_W {
            if bits & (0x80u8 >> col) == 0 {
                continue;
            }
            let gx = px0 + col * scale;
            let gy = py0 + row * scale;
            for sy in 0..scale {
                for sx in 0..scale {
                    put_pixel(gx + sx, gy + sy, fgp);
                }
            }
        }
    }
}

pub fn draw_cursor(cx: u32, cy: u32, attr: u8, visible: bool) {
    if !is_active() || !visible || cx >= cols() || cy >= rows() {
        return;
    }
    let fgp = pack(PALETTE[(attr & 0x0F) as usize]);
    let cw = cell_w();
    let chh = cell_h();
    let px0 = unsafe { ORIGIN_X } + cx * cw;
    let py0 = unsafe { ORIGIN_Y } + cy * chh;
    for col in 0..cw {
        put_pixel(px0 + col, py0 + chh - 1, fgp);
        if chh > 1 {
            put_pixel(px0 + col, py0 + chh - 2, fgp);
        }
    }
}

pub fn read(_buf: &mut [u8]) -> i64 {
    0
}

pub fn write(buf: &[u8]) -> i64 {
    buf.len() as i64
}

pub fn ioctl(request: u64, argp: *mut u8) -> i64 {
    match request {
        0x540B => {
            if !argp.is_null() {
                unsafe {
                    let w = argp as *mut u16;
                    *w = rows() as u16;
                    *w.add(1) = cols() as u16;
                    *w.add(2) = 0;
                    *w.add(3) = 0;
                }
            }
            0
        }
        0x4600 => 0,
        _ => -2,
    }
}
