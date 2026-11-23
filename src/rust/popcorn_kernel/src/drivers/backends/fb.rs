//! Framebuffer /dev/fb0 — full-bleed GOP text at native 8×16 glyphs.
//!
//! The GOP framebuffer is mapped uncached (boot_identity_uncache_range), so
//! every store to it is a bus transaction and loads are slower still. To keep
//! scrolling cheap on real hardware and under emulation:
//!
//! * Once the heap is up (`enable_shadow`), glyphs are rendered into a
//!   write-back RAM shadow of the text area. Scrolling is a memmove inside the
//!   shadow. `flush` pushes only changed pixel rows to the framebuffer with
//!   wide (u64) stores; full-width rows are skipped when their hash matches
//!   what was last pushed there. The framebuffer is never read back.
//! * Before the heap exists, cells are rendered into a small stack buffer and
//!   blitted one 8-pixel glyph row at a time (no per-byte stores).

use super::font8x16::FONT8X16;

pub const FONT_W: u32 = 8;
pub const FONT_H: u32 = 16;
pub const MAX_COLS: u32 = 240;
pub const MAX_ROWS: u32 = 70;
const POPCORN_FB_BG_RGB: u32 = 0x001018;
const MAX_TEXT_PX_ROWS: usize = (MAX_ROWS * FONT_H) as usize;
/// Hash value meaning "framebuffer row content unknown" (hash_row never returns 0).
const HASH_UNKNOWN: u64 = 0;

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

/* Write-back shadow of the text area (COLS*8 x ROWS*16 pixels, fb pixel format). */
static mut SHADOW: *mut u8 = core::ptr::null_mut();
static mut SHADOW_BYTES: usize = 0;
static mut SH_STRIDE: usize = 0;
static mut SH_ROWS: usize = 0;
/* Per text pixel row: dirty byte span [DMIN, DMAX) and hash of the fb row. */
static mut DMIN: [u32; MAX_TEXT_PX_ROWS] = [u32::MAX; MAX_TEXT_PX_ROWS];
static mut DMAX: [u32; MAX_TEXT_PX_ROWS] = [0; MAX_TEXT_PX_ROWS];
static mut FB_HASH: [u64; MAX_TEXT_PX_ROWS] = [HASH_UNKNOWN; MAX_TEXT_PX_ROWS];
static mut DIRTY_LO: usize = usize::MAX;
static mut DIRTY_HI: usize = 0;

extern "C" {
    /// kernel.asm low-identity PDPT (512 entries); only entry 0 is read here.
    static page_table_l3: [u64; 512];
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
    /* Low identity map: 512 GiB of 1 GiB pages (UEFI path with PDPE1GB, PS set in
     * page_table_l3[0]); otherwise the 2 MiB layout covers only 64 GiB (GRUB path,
     * or UEFI on CPUs without PDPE1GB). Anything past that would be unmapped. */
    let l3_0 = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(page_table_l3).cast::<u64>()) };
    let mapped_top = if (l3_0 & 0x80) != 0 { 512u64 << 30 } else { 64u64 << 30 };
    if h.addr.checked_add(span).map_or(true, |end| end > mapped_top) {
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

fn compute_layout() {
    unsafe {
        /* Native 8×16 → ~160×45 on 720p / ~240×67 on 1080p. */
        let cw = FONT_W;
        let ch = FONT_H;
        /* Never lay out more cells than fit on screen (rows past HEIGHT would
         * be clipped, hiding the status/heartbeat rows). The 320x200 handoff
         * floor already guarantees >= 40 cols and >= 12 rows. Keep the fb even
         * on tiny modes: UEFI-only machines have no VGA text to fall back to. */
        let mut c = WIDTH / cw;
        let mut r = HEIGHT / ch;
        if c < 1 {
            c = 1;
        }
        if r < 3 {
            r = 3;
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
        /* A shadow sized for another layout is unusable; drop back to direct
         * drawing (the buffer is leaked; layouts do not change in practice). */
        if !SHADOW.is_null() {
            let stride = (c * cw) as usize * BYTESPP as usize;
            let rows_px = (r * ch) as usize;
            if stride != SH_STRIDE || rows_px != SH_ROWS {
                SHADOW = core::ptr::null_mut();
            }
        }
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

/// Store one packed pixel (`bpp` bytes, little-endian) into cached memory.
#[inline(always)]
unsafe fn put_px(p: *mut u8, bpp: usize, pixel: u32) {
    if bpp == 4 {
        (p as *mut u32).write_unaligned(pixel);
    } else {
        for i in 0..bpp {
            *p.add(i) = (pixel >> (i * 8)) as u8;
        }
    }
}

/// Copy `len` bytes from cached RAM to the (uncached) framebuffer using the
/// widest volatile stores the alignment allows. Never reads `dst`.
unsafe fn blit(dst: *mut u8, src: *const u8, len: usize) {
    let mix = (dst as usize) | (src as usize) | len;
    if mix & 7 == 0 {
        let d = dst as *mut u64;
        let s = src as *const u64;
        for i in 0..len / 8 {
            core::ptr::write_volatile(d.add(i), *s.add(i));
        }
    } else if mix & 3 == 0 {
        let d = dst as *mut u32;
        let s = src as *const u32;
        for i in 0..len / 4 {
            core::ptr::write_volatile(d.add(i), *s.add(i));
        }
    } else {
        for i in 0..len {
            core::ptr::write_volatile(dst.add(i), *src.add(i));
        }
    }
}

/// Framebuffer address of text-area pixel (x, y) (text-area coordinates).
unsafe fn fb_text_ptr(x: usize, y: usize) -> *mut u8 {
    (BASE as *mut u8).add(
        (ORIGIN_Y as usize + y) * PITCH as usize + (ORIGIN_X as usize + x) * BYTESPP as usize,
    )
}

fn hash_row(p: *const u8, len: usize) -> u64 {
    /* len is a multiple of 8 (text width is a multiple of 8 pixels) and the
     * shadow is page aligned, so whole-word reads are in bounds and aligned. */
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    unsafe {
        let w = p as *const u64;
        for i in 0..len / 8 {
            h = (h.rotate_left(5) ^ *w.add(i)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h | 1
}

fn shadow_on() -> bool {
    unsafe { !SHADOW.is_null() }
}

unsafe fn mark_span(y: usize, x0: u32, x1: u32) {
    if y >= SH_ROWS {
        return;
    }
    if x0 < DMIN[y] {
        DMIN[y] = x0;
    }
    if x1 > DMAX[y] {
        DMAX[y] = x1;
    }
    if y < DIRTY_LO {
        DIRTY_LO = y;
    }
    if y + 1 > DIRTY_HI {
        DIRTY_HI = y + 1;
    }
}

/// Allocate the write-back text shadow once the kernel heap is up. Returns
/// true when the shadow is active. The caller must re-render every cell
/// (the shadow starts as solid background).
pub fn enable_shadow() -> bool {
    if !is_active() {
        return false;
    }
    if shadow_on() {
        return true;
    }
    unsafe {
        let stride = (COLS * FONT_W) as usize * BYTESPP as usize;
        let rows_px = (ROWS * FONT_H) as usize;
        if rows_px > MAX_TEXT_PX_ROWS || stride == 0 || stride % 8 != 0 {
            return false;
        }
        let bytes = stride * rows_px;
        let layout = match core::alloc::Layout::from_size_align(bytes, 4096) {
            Ok(l) => l,
            Err(_) => return false,
        };
        let p = alloc::alloc::alloc(layout);
        if p.is_null() {
            return false;
        }
        SH_STRIDE = stride;
        SH_ROWS = rows_px;
        SHADOW_BYTES = bytes;
        SHADOW = p;
        fill_shadow(pack(POPCORN_FB_BG_RGB));
        for y in 0..MAX_TEXT_PX_ROWS {
            DMIN[y] = u32::MAX;
            DMAX[y] = 0;
            FB_HASH[y] = HASH_UNKNOWN;
        }
        DIRTY_LO = usize::MAX;
        DIRTY_HI = 0;
    }
    true
}

unsafe fn fill_shadow(pix: u32) {
    if SHADOW.is_null() {
        return;
    }
    let bpp = BYTESPP as usize;
    let row0 = SHADOW;
    let mut x = 0;
    while x < SH_STRIDE {
        put_px(row0.add(x), bpp, pix);
        x += bpp;
    }
    for y in 1..SH_ROWS {
        core::ptr::copy_nonoverlapping(row0, SHADOW.add(y * SH_STRIDE), SH_STRIDE);
    }
}

/// Minimum TSC cycles between paced pushes (~5-20 ms at 1-4 GHz; no
/// calibration needed). The window also grows to 4x the cost of the last
/// push, so uncached fb writes stay under ~20% of a long output burst on
/// slow framebuffers.
const PUSH_WINDOW_TSC: u64 = 20_000_000;
static mut LAST_PUSH_TSC: u64 = 0;
static mut LAST_PUSH_COST: u64 = 0;

fn has_dirty() -> bool {
    unsafe { !SHADOW.is_null() && DIRTY_LO < DIRTY_HI }
}

/// Like `flush`, but coalesce bursts: if a push happened within the last
/// window, leave the rows dirty for the next push (`flush` from the idle loop
/// or `present`, or a later paced call). Each push to the uncached fb costs up
/// to a full text area per scrolled line, so a 40-line command output becomes
/// a couple of pushes instead of 40.
pub fn flush_paced() {
    if !has_dirty() {
        return;
    }
    let now = unsafe { core::arch::x86_64::_rdtsc() };
    unsafe {
        let mut window = LAST_PUSH_COST.saturating_mul(4);
        if window < PUSH_WINDOW_TSC {
            window = PUSH_WINDOW_TSC;
        }
        if now.wrapping_sub(LAST_PUSH_TSC) < window {
            return;
        }
    }
    flush();
}

/// Push every changed shadow row to the framebuffer.
pub fn flush() {
    if has_dirty() {
        unsafe {
            let t0 = core::arch::x86_64::_rdtsc();
            flush_rows();
            LAST_PUSH_TSC = core::arch::x86_64::_rdtsc();
            LAST_PUSH_COST = LAST_PUSH_TSC.wrapping_sub(t0);
        }
    }
}

fn flush_rows() {
    if !is_active() || !shadow_on() {
        return;
    }
    unsafe {
        if DIRTY_LO >= DIRTY_HI {
            return;
        }
        let full = SH_STRIDE as u32;
        let hi = if DIRTY_HI > SH_ROWS { SH_ROWS } else { DIRTY_HI };
        for y in DIRTY_LO..hi {
            let (a, b) = (DMIN[y], DMAX[y]);
            if a >= b {
                continue;
            }
            DMIN[y] = u32::MAX;
            DMAX[y] = 0;
            let src = SHADOW.add(y * SH_STRIDE);
            if a == 0 && b >= full {
                let h = hash_row(src, SH_STRIDE);
                if h != FB_HASH[y] {
                    blit(fb_text_ptr(0, y), src, SH_STRIDE);
                    FB_HASH[y] = h;
                }
            } else {
                let b = if b > full { full } else { b };
                blit(fb_text_ptr(0, y).add(a as usize), src.add(a as usize), (b - a) as usize);
                FB_HASH[y] = HASH_UNKNOWN;
            }
        }
        DIRTY_LO = usize::MAX;
        DIRTY_HI = 0;
    }
}

/// Scroll text rows [row0, row0+n) up by one cell row inside the shadow. The
/// bottom row's pixels are left stale; the caller redraws its cells. Returns
/// false when there is no shadow (caller falls back to redrawing cells).
pub fn scroll_up(row0: u32, n: u32) -> bool {
    if !is_active() || !shadow_on() || n < 2 || row0 + n > rows() {
        return false;
    }
    unsafe {
        let band = FONT_H as usize * SH_STRIDE;
        let dst = SHADOW.add(row0 as usize * band);
        core::ptr::copy(dst.add(band), dst, (n as usize - 1) * band);
        let full = SH_STRIDE as u32;
        let y0 = (row0 * FONT_H) as usize;
        let y1 = ((row0 + n - 1) * FONT_H) as usize;
        for y in y0..y1 {
            mark_span(y, 0, full);
        }
    }
    true
}

/// Fill the whole framebuffer with `rgb` (0xRRGGBB), packed for the active
/// channel layout (RGB or BGR GOP, 15/16/24/32 bpp).
pub fn paint_background(rgb: u32) {
    if !is_active() {
        return;
    }
    let pix = pack(rgb);
    unsafe {
        if BASE == 0 {
            return;
        }
        let bpp = BYTESPP as usize;
        /* Build one solid row in RAM, then blit it to every scanline. */
        let mut line = [0u8; 64];
        let mut i = 0;
        while i + bpp <= 64 {
            put_px(line.as_mut_ptr().add(i), bpp, pix);
            i += bpp;
        }
        let chunk = (64 / bpp) * bpp; /* whole pixels per chunk */
        let row_bytes = WIDTH as usize * bpp;
        for y in 0..HEIGHT as usize {
            let row = (BASE as *mut u8).add(y * PITCH as usize);
            let mut off = 0;
            while off < row_bytes {
                let n = if row_bytes - off < chunk { row_bytes - off } else { chunk };
                blit(row.add(off), line.as_ptr(), n);
                off += n;
            }
        }
        if shadow_on() {
            fill_shadow(pix);
            for y in 0..SH_ROWS {
                DMIN[y] = u32::MAX;
                DMAX[y] = 0;
                FB_HASH[y] = HASH_UNKNOWN;
            }
            DIRTY_LO = usize::MAX;
            DIRTY_HI = 0;
        }
    }
}

/// Clear the text panel and margins to the console background. Panel and
/// margins share one packed color, so a single full-screen fill covers both.
pub fn fill_text_panel() {
    if !is_active() {
        return;
    }
    paint_background(POPCORN_FB_BG_RGB);
}

pub fn relayout() {
    if !is_active() {
        return;
    }
    compute_layout();
    fill_text_panel();
}

/// Render one glyph row (8 pixels) into `dst` (cached memory).
#[inline(always)]
unsafe fn render_glyph_row(dst: *mut u8, bpp: usize, bits: u8, fgp: u32, bgp: u32) {
    for col in 0..FONT_W as usize {
        let px = if bits & (0x80u8 >> col) != 0 { fgp } else { bgp };
        put_px(dst.add(col * bpp), bpp, px);
    }
}

pub fn draw_cell(cx: u32, cy: u32, ch: u8, attr: u8) {
    if !is_active() || cx >= cols() || cy >= rows() {
        return;
    }
    let fgp = pack(PALETTE[(attr & 0x0F) as usize]);
    let bgp = pack(PALETTE[((attr >> 4) & 0x07) as usize]);
    let glyph = &FONT8X16[(ch & 0x7F) as usize];
    unsafe {
        let bpp = BYTESPP as usize;
        let row_bytes = FONT_W as usize * bpp;
        let x0 = cx as usize * FONT_W as usize;
        let y0 = cy as usize * FONT_H as usize;
        if shadow_on() {
            let xb = (x0 * bpp) as u32;
            for row in 0..FONT_H as usize {
                let dst = SHADOW.add((y0 + row) * SH_STRIDE + x0 * bpp);
                render_glyph_row(dst, bpp, glyph[row], fgp, bgp);
                mark_span(y0 + row, xb, xb + row_bytes as u32);
            }
        } else {
            let mut buf = [0u64; 4]; /* 8 px * up to 4 bytes */
            let bp = buf.as_mut_ptr() as *mut u8;
            for row in 0..FONT_H as usize {
                render_glyph_row(bp, bpp, glyph[row], fgp, bgp);
                blit(fb_text_ptr(x0, y0 + row), bp, row_bytes);
            }
        }
    }
}

pub fn draw_cursor(cx: u32, cy: u32, attr: u8, visible: bool) {
    if !is_active() || !visible || cx >= cols() || cy >= rows() {
        return;
    }
    let fgp = pack(PALETTE[(attr & 0x0F) as usize]);
    unsafe {
        let bpp = BYTESPP as usize;
        let row_bytes = FONT_W as usize * bpp;
        let x0 = cx as usize * FONT_W as usize;
        let y0 = cy as usize * FONT_H as usize;
        let mut buf = [0u64; 4];
        let bp = buf.as_mut_ptr() as *mut u8;
        render_glyph_row(bp, bpp, 0xFF, fgp, fgp);
        for row in (FONT_H as usize - 2)..FONT_H as usize {
            if shadow_on() {
                let dst = SHADOW.add((y0 + row) * SH_STRIDE + x0 * bpp);
                core::ptr::copy_nonoverlapping(bp, dst, row_bytes);
                let xb = (x0 * bpp) as u32;
                mark_span(y0 + row, xb, xb + row_bytes as u32);
            } else {
                blit(fb_text_ptr(x0, y0 + row), bp, row_bytes);
            }
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
