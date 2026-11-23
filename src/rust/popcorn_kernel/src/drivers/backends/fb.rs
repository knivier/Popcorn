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
//!
//! Synchronisation: the mode geometry and pixel format are independent
//! scalars written only at handoff/relayout, kept in relaxed atomics. The
//! shadow buffer and its dirty tracking are updated together and live in
//! [`FbState`] behind one `IrqSpinLock` ([`lock`]). Drawing goes through a
//! held guard; when the screen layer needs both, it takes its own lock first.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering::Relaxed};

use super::font8x16::FONT8X16;
use crate::sync::{IrqGuard, IrqSpinLock};

pub const FONT_W: u32 = 8;
pub const FONT_H: u32 = 16;
pub const MAX_COLS: u32 = 240;
pub const MAX_ROWS: u32 = 70;
const POPCORN_FB_BG_RGB: u32 = 0x001018;
const MAX_TEXT_PX_ROWS: usize = (MAX_ROWS * FONT_H) as usize;
/// Hash value meaning "framebuffer row content unknown" (hash_row never returns 0).
const HASH_UNKNOWN: u64 = 0;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static BASE: AtomicUsize = AtomicUsize::new(0);
static PITCH: AtomicU32 = AtomicU32::new(0);
static WIDTH: AtomicU32 = AtomicU32::new(0);
static HEIGHT: AtomicU32 = AtomicU32::new(0);
static BYTESPP: AtomicU8 = AtomicU8::new(0);
static RPOS: AtomicU8 = AtomicU8::new(0);
static RSIZE: AtomicU8 = AtomicU8::new(8);
static GPOS: AtomicU8 = AtomicU8::new(0);
static GSIZE: AtomicU8 = AtomicU8::new(8);
static BPOS: AtomicU8 = AtomicU8::new(0);
static BSIZE: AtomicU8 = AtomicU8::new(8);
static COLS: AtomicU32 = AtomicU32::new(80);
static ROWS: AtomicU32 = AtomicU32::new(25);
static ORIGIN_X: AtomicU32 = AtomicU32::new(0);
static ORIGIN_Y: AtomicU32 = AtomicU32::new(0);

/// Write-back shadow of the text area (COLS*8 x ROWS*16 pixels, fb pixel
/// format) plus its dirty tracking and push pacing.
pub struct FbState {
    shadow: *mut u8,
    sh_stride: usize,
    sh_rows: usize,
    /* Per text pixel row: dirty byte span [dmin, dmax) and hash of the fb row. */
    dmin: [u32; MAX_TEXT_PX_ROWS],
    dmax: [u32; MAX_TEXT_PX_ROWS],
    fb_hash: [u64; MAX_TEXT_PX_ROWS],
    dirty_lo: usize,
    dirty_hi: usize,
    last_push_tsc: u64,
    last_push_cost: u64,
}

// Safety: `shadow` is a heap buffer owned by this state (leaked, never freed
// once installed) and is only dereferenced while the STATE lock is held.
unsafe impl Send for FbState {}

static STATE: IrqSpinLock<FbState> = IrqSpinLock::new(FbState {
    shadow: core::ptr::null_mut(),
    sh_stride: 0,
    sh_rows: 0,
    dmin: [u32::MAX; MAX_TEXT_PX_ROWS],
    dmax: [0; MAX_TEXT_PX_ROWS],
    fb_hash: [HASH_UNKNOWN; MAX_TEXT_PX_ROWS],
    dirty_lo: usize::MAX,
    dirty_hi: 0,
    last_push_tsc: 0,
    last_push_cost: 0,
});

/// Lock the shadow state (interrupts off while held). Lock order: the screen
/// state lock, if needed, must be taken before this one.
#[track_caller]
#[inline]
pub fn lock() -> IrqGuard<'static, FbState> {
    STATE.lock()
}

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
    ACTIVE.load(Relaxed)
}

pub fn cols() -> u32 {
    COLS.load(Relaxed)
}

pub fn rows() -> u32 {
    ROWS.load(Relaxed)
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

    unsafe { boot_identity_uncache_range(h.addr, span) };
    BASE.store(h.addr as usize, Relaxed);
    PITCH.store(pitch, Relaxed);
    WIDTH.store(h.width, Relaxed);
    HEIGHT.store(h.height, Relaxed);
    BYTESPP.store(bytespp, Relaxed);
    RPOS.store(rpos, Relaxed);
    RSIZE.store(rsize, Relaxed);
    GPOS.store(gpos, Relaxed);
    GSIZE.store(gsize, Relaxed);
    BPOS.store(bpos, Relaxed);
    BSIZE.store(bsize, Relaxed);
    ACTIVE.store(true, Relaxed);
    compute_layout(&mut STATE.lock());
    true
}

fn compute_layout(s: &mut FbState) {
    /* Native 8×16 → ~160×45 on 720p / ~240×67 on 1080p. */
    let cw = FONT_W;
    let ch = FONT_H;
    let width = WIDTH.load(Relaxed);
    let height = HEIGHT.load(Relaxed);
    /* Never lay out more cells than fit on screen (rows past HEIGHT would
     * be clipped, hiding the status/heartbeat rows). The 320x200 handoff
     * floor already guarantees >= 40 cols and >= 12 rows. Keep the fb even
     * on tiny modes: UEFI-only machines have no VGA text to fall back to. */
    let mut c = width / cw;
    let mut r = height / ch;
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
    COLS.store(c, Relaxed);
    ROWS.store(r, Relaxed);
    let text_w = c * cw;
    let text_h = r * ch;
    ORIGIN_X.store(if width > text_w { (width - text_w) / 2 } else { 0 }, Relaxed);
    ORIGIN_Y.store(if height > text_h { (height - text_h) / 2 } else { 0 }, Relaxed);
    /* A shadow sized for another layout is unusable; drop back to direct
     * drawing (the buffer is leaked; layouts do not change in practice). */
    if !s.shadow.is_null() {
        let stride = (c * cw) as usize * BYTESPP.load(Relaxed) as usize;
        let rows_px = (r * ch) as usize;
        if stride != s.sh_stride || rows_px != s.sh_rows {
            s.shadow = core::ptr::null_mut();
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
    let r = scale_chan((rgb >> 16) & 0xFF, RSIZE.load(Relaxed));
    let g = scale_chan((rgb >> 8) & 0xFF, GSIZE.load(Relaxed));
    let b = scale_chan(rgb & 0xFF, BSIZE.load(Relaxed));
    (r << RPOS.load(Relaxed)) | (g << GPOS.load(Relaxed)) | (b << BPOS.load(Relaxed))
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
    (BASE.load(Relaxed) as *mut u8).add(
        (ORIGIN_Y.load(Relaxed) as usize + y) * PITCH.load(Relaxed) as usize
            + (ORIGIN_X.load(Relaxed) as usize + x) * BYTESPP.load(Relaxed) as usize,
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

/// Allocate the write-back text shadow once the kernel heap is up. Returns
/// true when the shadow is active. The caller must re-render every cell
/// (the shadow starts as solid background). Must not be called with the
/// fb lock held: the allocation happens outside it.
pub fn enable_shadow() -> bool {
    if !is_active() {
        return false;
    }
    if STATE.lock().shadow_on() {
        return true;
    }
    let stride = (cols() * FONT_W) as usize * BYTESPP.load(Relaxed) as usize;
    let rows_px = (rows() * FONT_H) as usize;
    if rows_px > MAX_TEXT_PX_ROWS || stride == 0 || stride % 8 != 0 {
        return false;
    }
    let bytes = stride * rows_px;
    let layout = match core::alloc::Layout::from_size_align(bytes, 4096) {
        Ok(l) => l,
        Err(_) => return false,
    };
    let p = unsafe { alloc::alloc::alloc(layout) };
    if p.is_null() {
        return false;
    }
    let pix = pack(POPCORN_FB_BG_RGB);
    let mut s = STATE.lock();
    if s.shadow_on() {
        /* Lost a race with another enabler: keep theirs. */
        drop(s);
        unsafe { alloc::alloc::dealloc(p, layout) };
        return true;
    }
    s.sh_stride = stride;
    s.sh_rows = rows_px;
    s.shadow = p;
    s.fill_shadow(pix);
    s.reset_rows(MAX_TEXT_PX_ROWS);
    true
}

/// Minimum TSC cycles between paced pushes (~5-20 ms at 1-4 GHz; no
/// calibration needed). The window also grows to 4x the cost of the last
/// push, so uncached fb writes stay under ~20% of a long output burst on
/// slow framebuffers.
const PUSH_WINDOW_TSC: u64 = 20_000_000;

/// Render one glyph row (8 pixels) into `dst` (cached memory).
#[inline(always)]
unsafe fn render_glyph_row(dst: *mut u8, bpp: usize, bits: u8, fgp: u32, bgp: u32) {
    for col in 0..FONT_W as usize {
        let px = if bits & (0x80u8 >> col) != 0 { fgp } else { bgp };
        put_px(dst.add(col * bpp), bpp, px);
    }
}

impl FbState {
    #[inline]
    fn shadow_on(&self) -> bool {
        !self.shadow.is_null()
    }

    #[inline]
    fn mark_span(&mut self, y: usize, x0: u32, x1: u32) {
        if y >= self.sh_rows {
            return;
        }
        if x0 < self.dmin[y] {
            self.dmin[y] = x0;
        }
        if x1 > self.dmax[y] {
            self.dmax[y] = x1;
        }
        if y < self.dirty_lo {
            self.dirty_lo = y;
        }
        if y + 1 > self.dirty_hi {
            self.dirty_hi = y + 1;
        }
    }

    /// Forget dirty spans and fb hashes for pixel rows [0, n).
    fn reset_rows(&mut self, n: usize) {
        for y in 0..n {
            self.dmin[y] = u32::MAX;
            self.dmax[y] = 0;
            self.fb_hash[y] = HASH_UNKNOWN;
        }
        self.dirty_lo = usize::MAX;
        self.dirty_hi = 0;
    }

    fn fill_shadow(&mut self, pix: u32) {
        if self.shadow.is_null() {
            return;
        }
        let bpp = BYTESPP.load(Relaxed) as usize;
        let row0 = self.shadow;
        unsafe {
            let mut x = 0;
            while x < self.sh_stride {
                put_px(row0.add(x), bpp, pix);
                x += bpp;
            }
            for y in 1..self.sh_rows {
                core::ptr::copy_nonoverlapping(row0, self.shadow.add(y * self.sh_stride), self.sh_stride);
            }
        }
    }

    #[inline]
    fn has_dirty(&self) -> bool {
        !self.shadow.is_null() && self.dirty_lo < self.dirty_hi
    }

    /// Like `flush`, but coalesce bursts: if a push happened within the last
    /// window, leave the rows dirty for the next push (`flush` from the idle loop
    /// or `present`, or a later paced call). Each push to the uncached fb costs up
    /// to a full text area per scrolled line, so a 40-line command output becomes
    /// a couple of pushes instead of 40.
    pub fn flush_paced(&mut self) {
        if !self.has_dirty() {
            return;
        }
        let now = unsafe { core::arch::x86_64::_rdtsc() };
        let mut window = self.last_push_cost.saturating_mul(4);
        if window < PUSH_WINDOW_TSC {
            window = PUSH_WINDOW_TSC;
        }
        if now.wrapping_sub(self.last_push_tsc) < window {
            return;
        }
        self.flush();
    }

    /// Push every changed shadow row to the framebuffer.
    pub fn flush(&mut self) {
        if self.has_dirty() {
            let t0 = unsafe { core::arch::x86_64::_rdtsc() };
            self.flush_rows();
            self.last_push_tsc = unsafe { core::arch::x86_64::_rdtsc() };
            self.last_push_cost = self.last_push_tsc.wrapping_sub(t0);
        }
    }

    fn flush_rows(&mut self) {
        if !is_active() || !self.shadow_on() {
            return;
        }
        if self.dirty_lo >= self.dirty_hi {
            return;
        }
        let stride = self.sh_stride;
        let full = stride as u32;
        let hi = if self.dirty_hi > self.sh_rows { self.sh_rows } else { self.dirty_hi };
        for y in self.dirty_lo..hi {
            let (a, b) = (self.dmin[y], self.dmax[y]);
            if a >= b {
                continue;
            }
            self.dmin[y] = u32::MAX;
            self.dmax[y] = 0;
            unsafe {
                let src = self.shadow.add(y * stride);
                if a == 0 && b >= full {
                    let h = hash_row(src, stride);
                    if h != self.fb_hash[y] {
                        blit(fb_text_ptr(0, y), src, stride);
                        self.fb_hash[y] = h;
                    }
                } else {
                    let b = if b > full { full } else { b };
                    blit(fb_text_ptr(0, y).add(a as usize), src.add(a as usize), (b - a) as usize);
                    self.fb_hash[y] = HASH_UNKNOWN;
                }
            }
        }
        self.dirty_lo = usize::MAX;
        self.dirty_hi = 0;
    }

    /// Scroll text rows [row0, row0+n) up by one cell row inside the shadow. The
    /// bottom row's pixels are left stale; the caller redraws its cells. Returns
    /// false when there is no shadow (caller falls back to redrawing cells).
    pub fn scroll_up(&mut self, row0: u32, n: u32) -> bool {
        if !is_active() || !self.shadow_on() || n < 2 || row0 + n > rows() {
            return false;
        }
        let band = FONT_H as usize * self.sh_stride;
        unsafe {
            let dst = self.shadow.add(row0 as usize * band);
            core::ptr::copy(dst.add(band), dst, (n as usize - 1) * band);
        }
        let full = self.sh_stride as u32;
        let y0 = (row0 * FONT_H) as usize;
        let y1 = ((row0 + n - 1) * FONT_H) as usize;
        for y in y0..y1 {
            self.mark_span(y, 0, full);
        }
        true
    }

    /// Fill the whole framebuffer with `rgb` (0xRRGGBB), packed for the active
    /// channel layout (RGB or BGR GOP, 15/16/24/32 bpp).
    pub fn paint_background(&mut self, rgb: u32) {
        if !is_active() {
            return;
        }
        let pix = pack(rgb);
        let base = BASE.load(Relaxed);
        if base == 0 {
            return;
        }
        let bpp = BYTESPP.load(Relaxed) as usize;
        let pitch = PITCH.load(Relaxed) as usize;
        /* Build one solid row in RAM, then blit it to every scanline. */
        let mut line = [0u8; 64];
        let mut i = 0;
        while i + bpp <= 64 {
            unsafe { put_px(line.as_mut_ptr().add(i), bpp, pix) };
            i += bpp;
        }
        let chunk = (64 / bpp) * bpp; /* whole pixels per chunk */
        let row_bytes = WIDTH.load(Relaxed) as usize * bpp;
        for y in 0..HEIGHT.load(Relaxed) as usize {
            unsafe {
                let row = (base as *mut u8).add(y * pitch);
                let mut off = 0;
                while off < row_bytes {
                    let n = if row_bytes - off < chunk { row_bytes - off } else { chunk };
                    blit(row.add(off), line.as_ptr(), n);
                    off += n;
                }
            }
        }
        if self.shadow_on() {
            self.fill_shadow(pix);
            let n = self.sh_rows;
            self.reset_rows(n);
        }
    }

    /// Clear the text panel and margins to the console background. Panel and
    /// margins share one packed color, so a single full-screen fill covers both.
    pub fn fill_text_panel(&mut self) {
        if !is_active() {
            return;
        }
        self.paint_background(POPCORN_FB_BG_RGB);
    }

    pub fn relayout(&mut self) {
        if !is_active() {
            return;
        }
        compute_layout(self);
        self.fill_text_panel();
    }

    pub fn draw_cell(&mut self, cx: u32, cy: u32, ch: u8, attr: u8) {
        if !is_active() || cx >= cols() || cy >= rows() {
            return;
        }
        let fgp = pack(PALETTE[(attr & 0x0F) as usize]);
        let bgp = pack(PALETTE[((attr >> 4) & 0x07) as usize]);
        let glyph = &FONT8X16[(ch & 0x7F) as usize];
        let bpp = BYTESPP.load(Relaxed) as usize;
        let row_bytes = FONT_W as usize * bpp;
        let x0 = cx as usize * FONT_W as usize;
        let y0 = cy as usize * FONT_H as usize;
        unsafe {
            if self.shadow_on() {
                let xb = (x0 * bpp) as u32;
                for row in 0..FONT_H as usize {
                    let dst = self.shadow.add((y0 + row) * self.sh_stride + x0 * bpp);
                    render_glyph_row(dst, bpp, glyph[row], fgp, bgp);
                    self.mark_span(y0 + row, xb, xb + row_bytes as u32);
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

    pub fn draw_cursor(&mut self, cx: u32, cy: u32, attr: u8, visible: bool) {
        if !is_active() || !visible || cx >= cols() || cy >= rows() {
            return;
        }
        let fgp = pack(PALETTE[(attr & 0x0F) as usize]);
        let bpp = BYTESPP.load(Relaxed) as usize;
        let row_bytes = FONT_W as usize * bpp;
        let x0 = cx as usize * FONT_W as usize;
        let y0 = cy as usize * FONT_H as usize;
        let mut buf = [0u64; 4];
        let bp = buf.as_mut_ptr() as *mut u8;
        unsafe {
            render_glyph_row(bp, bpp, 0xFF, fgp, fgp);
            for row in (FONT_H as usize - 2)..FONT_H as usize {
                if self.shadow_on() {
                    let dst = self.shadow.add((y0 + row) * self.sh_stride + x0 * bpp);
                    core::ptr::copy_nonoverlapping(bp, dst, row_bytes);
                    let xb = (x0 * bpp) as u32;
                    self.mark_span(y0 + row, xb, xb + row_bytes as u32);
                } else {
                    blit(fb_text_ptr(x0, y0 + row), bp, row_bytes);
                }
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
