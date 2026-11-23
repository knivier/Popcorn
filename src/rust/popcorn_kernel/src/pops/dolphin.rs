//! Dolphin text editor pop (ported from C).

use super::registry::PopModule;
use crate::console_ffi::{
    self, print, print_color, print_u32, println, println_color, COLOR_HEADER, COLOR_INFO,
    COLOR_PROMPT, COLOR_SUCCESS, COLOR_WARNING, COLOR_WHITE,
};
use crate::sync::TaskLock;
use core::ffi::c_char;

const MAX_LINES: usize = 100;
const MAX_LINE_LENGTH: usize = 80;
const EDITOR_DISPLAY_LINES: usize = 20;
/// Whole editor buffer: every line holds at most MAX_LINE_LENGTH-1 chars plus a
/// '\n' separator, then the NUL. Well under fat32::MAX_CONTENT, so saves never
/// have to truncate.
const SAVE_BUF_LEN: usize = MAX_LINES * MAX_LINE_LENGTH + 1;

const KEY_ENTER: u8 = 0x1C;
const KEY_BACKSPACE: u8 = 0x0E;
const KEY_ESC: u8 = 0x01;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_ESC_RELEASE: u8 = 0x81;
const KBD_SCAN_LSHIFT: u8 = 0x2A;
const KBD_SCAN_RSHIFT: u8 = 0x36;

static NAME: &[u8] = b"dolphin\0";
static MESSAGE: &[u8] = b"Dolphin text editor\0";

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum EditorMode {
    Normal = 0,
    Insert = 1,
    Command = 2,
}

struct Editor {
    lines: [[u8; MAX_LINE_LENGTH]; MAX_LINES],
    num_lines: usize,
    cursor_line: usize,
    cursor_col: usize,
    scroll_offset: usize,
    mode: EditorMode,
    modified: bool,
    filename: [u8; 64],
    active: bool,
}

impl Editor {
    const fn empty() -> Self {
        Self {
            lines: [[0; MAX_LINE_LENGTH]; MAX_LINES],
            num_lines: 0,
            cursor_line: 0,
            cursor_col: 0,
            scroll_offset: 0,
            mode: EditorMode::Normal,
            modified: false,
            filename: [0; 64],
            active: false,
        }
    }
}

struct DolphinState {
    ed: Editor,
    /// Serialization buffer for saves (~8 KiB: kept off the kernel stack). Only
    /// touched by `save_inner`.
    save_buf: [u8; SAVE_BUF_LEN],
}

/// Editor state. A TaskLock (not a spinlock): it is held across file saves/loads
/// (disk I/O) and while ESC command mode waits for keys with interrupts on.
///
/// Lock order: dolphin `EDITOR` → fat32 `FS` → block table. Dolphin calls the
/// filesystem (read_file / write_file / get_current_directory) while holding this
/// lock; the filesystem must never call back into dolphin. Re-entering from the
/// holder (e.g. a `dolphin_*` export called from inside an editor callback) panics.
static EDITOR: TaskLock<DolphinState> =
    TaskLock::new(DolphinState { ed: Editor::empty(), save_buf: [0; SAVE_BUF_LEN] });

fn with_editor<R>(f: impl FnOnce(&mut Editor) -> R) -> R {
    let mut g = EDITOR.lock();
    f(&mut g.ed)
}

/// Like `with_editor`, also lending the save buffer (paths that may save).
fn with_editor_save<R>(f: impl FnOnce(&mut Editor, &mut [u8; SAVE_BUF_LEN]) -> R) -> R {
    let mut g = EDITOR.lock();
    let DolphinState { ed, save_buf } = &mut *g;
    f(ed, save_buf)
}

extern "C" {
    fn write_file(name: *const c_char, content: *const c_char) -> bool;
    fn read_file(name: *const c_char) -> *const c_char;
    fn get_last_filesystem_error() -> i32;
    fn get_current_directory() -> *const c_char;
    fn key_queue_pop(out: *mut u8) -> bool;
    fn kbd_scancode_to_char(code: u8, shift: i32, caps: i32) -> i8;
    fn kbd_shift_active() -> i32;
    fn kbd_caps_active() -> i32;
    fn timer_is_poll_mode() -> bool;
}

fn cstr_len(s: &[u8]) -> usize {
    let mut i = 0;
    while i < s.len() && s[i] != 0 {
        i += 1;
    }
    i
}

fn cstr_eq(a: &[u8], b: &[u8]) -> bool {
    let n = cstr_len(a);
    let m = cstr_len(b);
    n == m && a[..n] == b[..m]
}

fn copy_cstr(dest: &mut [u8], src: &str) {
    let bytes = src.as_bytes();
    let n = bytes.len().min(dest.len().saturating_sub(1));
    dest[..n].copy_from_slice(&bytes[..n]);
    dest[n] = 0;
    for b in &mut dest[n + 1..] {
        *b = 0;
    }
}

/// Default a bare name to `.txt`. A name that already has any extension
/// (README.TXT, note.txt, notes.md) is left alone: FAT 8.3 allows only one dot,
/// and to_sfn reports names that do not fit instead of us mangling them.
fn ensure_txt(name: &mut [u8]) {
    let n = cstr_len(name);
    if name[..n].contains(&b'.') {
        return;
    }
    if n + 4 < name.len() {
        name[n..n + 4].copy_from_slice(b".txt");
        name[n + 4] = 0;
    }
}

fn line_len(line: &[u8]) -> usize {
    cstr_len(line)
}

fn from_c_str(p: *const c_char) -> &'static str {
    if p.is_null() {
        return "";
    }
    unsafe {
        let mut len = 0usize;
        while *p.add(len) != 0 && len < 256 {
            len += 1;
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(p as *const u8, len))
    }
}

fn filename_str(ed: &Editor) -> &str {
    let n = cstr_len(&ed.filename);
    core::str::from_utf8(&ed.filename[..n]).unwrap_or("")
}

fn init_ui(ed: &Editor) {
    console_ffi::clear();
    console_ffi::set_cursor(0, 0);
    println_color("=== Dolphin Text Editor ===", COLOR_HEADER);
    print_color("Editing: ", COLOR_INFO);
    println_color(filename_str(ed), COLOR_SUCCESS);
    console_ffi::separator_at(2);
}

fn restore_shell(msg: &str, warning: bool) {
    console_ffi::clear();
    console_ffi::header("Popcorn Kernel v0.7");
    if warning {
        println_color(msg, COLOR_WARNING);
    } else {
        console_ffi::success(msg);
    }
    console_ffi::newline();
    let path = unsafe { get_current_directory() };
    console_ffi::prompt_with_path(from_c_str(path));
}

fn close_clean(ed: &mut Editor) {
    let mut name_buf = [0u8; 64];
    name_buf.copy_from_slice(&ed.filename);
    ed.active = false;
    console_ffi::clear();
    console_ffi::header("Popcorn Kernel v0.7");
    console_ffi::success("Dolphin editor closed");
    print_color("File: ", COLOR_INFO);
    let n = cstr_len(&name_buf);
    println_color(
        core::str::from_utf8(&name_buf[..n]).unwrap_or(""),
        COLOR_WHITE,
    );
    console_ffi::newline();
    let path = unsafe { get_current_directory() };
    console_ffi::prompt_with_path(from_c_str(path));
}

#[no_mangle]
pub extern "C" fn dolphin_is_active() -> bool {
    with_editor(|ed| ed.active)
}

#[no_mangle]
pub extern "C" fn dolphin_force_quit() {
    with_editor(|ed| {
        if !ed.active {
            return;
        }
        ed.active = false;
        restore_shell("Dolphin editor closed (unsaved changes discarded)", true);
    });
}

#[no_mangle]
pub extern "C" fn dolphin_new(filename: *const c_char) {
    if filename.is_null() {
        console_ffi::error("Usage: dol -new <filename>");
        return;
    }
    let name = from_c_str(filename);
    if name.is_empty() {
        console_ffi::error("Usage: dol -new <filename>");
        return;
    }
    with_editor(|ed| {
        copy_cstr(&mut ed.filename, name);
        ensure_txt(&mut ed.filename);
        ed.num_lines = 1;
        for line in ed.lines.iter_mut() {
            line[0] = 0;
        }
        ed.cursor_line = 0;
        ed.cursor_col = 0;
        ed.scroll_offset = 0;
        ed.mode = EditorMode::Insert;
        ed.modified = false;
        ed.active = true;
        init_ui(ed);
        render(ed);
    });
}

#[no_mangle]
pub extern "C" fn dolphin_open(filename: *const c_char) {
    if filename.is_null() {
        console_ffi::error("Usage: dol -open <filename>");
        return;
    }
    let name = from_c_str(filename);
    if name.is_empty() {
        console_ffi::error("Usage: dol -open <filename>");
        return;
    }

    let mut fname = [0u8; 64];
    copy_cstr(&mut fname, name);
    ensure_txt(&mut fname);

    /* Read under the editor lock (order: editor → FS) and parse straight away:
     * read_file's buffer is only valid until the next read_file call. */
    with_editor(|ed| {
        let content = unsafe { read_file(fname.as_ptr() as *const c_char) };
        if content.is_null() {
            console_ffi::error("File not found. Use 'dol -new' to create it");
            return;
        }
        ed.filename = fname;
        ed.num_lines = 0;
        let mut line_pos = 0usize;
        let mut p = content;
        unsafe {
            while *p != 0 && ed.num_lines < MAX_LINES {
                let ch = *p as u8;
                if ch == b'\n' {
                    ed.lines[ed.num_lines][line_pos] = 0;
                    ed.num_lines += 1;
                    line_pos = 0;
                    p = p.add(1);
                } else if line_pos >= MAX_LINE_LENGTH - 1 {
                    /* Line full: wrap without consuming `ch`, so it becomes the
                     * first char of the next line instead of being dropped. */
                    ed.lines[ed.num_lines][line_pos] = 0;
                    ed.num_lines += 1;
                    line_pos = 0;
                } else {
                    ed.lines[ed.num_lines][line_pos] = ch;
                    line_pos += 1;
                    p = p.add(1);
                }
            }
        }
        if line_pos > 0 || ed.num_lines == 0 {
            ed.lines[ed.num_lines][line_pos] = 0;
            ed.num_lines += 1;
        }
        ed.cursor_line = 0;
        ed.cursor_col = 0;
        ed.scroll_offset = 0;
        ed.mode = EditorMode::Insert;
        ed.modified = false;
        ed.active = true;
        init_ui(ed);
        render(ed);
    });
}

#[no_mangle]
pub extern "C" fn dolphin_save() {
    with_editor_save(|ed, buf| {
        if !ed.active {
            console_ffi::error("No file open in editor");
            return;
        }
        save_inner(ed, buf, false);
    });
}

#[no_mangle]
pub extern "C" fn dolphin_close() {
    with_editor(|ed| {
        if !ed.active {
            return;
        }
        if ed.modified {
            console_ffi::warning("File has unsaved changes!");
            println_color(
                "Use 'dol -save' first or 'dol -quit!' to force quit",
                COLOR_INFO,
            );
            return;
        }
        close_clean(ed);
    });
}

#[no_mangle]
pub extern "C" fn dolphin_help() {
    console_ffi::newline();
    println_color("=== Dolphin Text Editor ===", COLOR_HEADER);
    console_ffi::separator();

    println_color("Commands:", COLOR_INFO);
    print_color("  dol -new <file>  ", COLOR_PROMPT);
    println(" - Create new text file");
    print_color("  dol -open <file> ", COLOR_PROMPT);
    println(" - Open existing text file");
    print_color("  dol -save        ", COLOR_PROMPT);
    println(" - Save current file (from shell)");
    print_color("  dol -close       ", COLOR_PROMPT);
    println(" - Close editor (from shell)");
    print_color("  dol -quit!       ", COLOR_PROMPT);
    println(" - Force quit without saving");
    print_color("  dol -help        ", COLOR_PROMPT);
    println(" - Show this help");

    console_ffi::newline();
    println_color("While editing:", COLOR_INFO);
    println(" • Type normally to insert text");
    println(" • Backspace to delete characters");
    println(" • Enter to create new line");

    console_ffi::newline();
    println_color("Commands (press ESC then type):", COLOR_INFO);
    println(" • w         - Save file");
    println(" • q         - Quit (fails if unsaved)");
    println(" • q!        - Force quit without saving");
    println(" • wq or x   - Save and quit");

    console_ffi::separator();
}

fn insert_char(ed: &mut Editor, ch: u8) {
    if !ed.active || ed.cursor_line >= ed.num_lines {
        return;
    }
    let line = &mut ed.lines[ed.cursor_line];
    let llen = line_len(line);
    if llen < MAX_LINE_LENGTH - 1 && ed.cursor_col <= llen {
        for i in (ed.cursor_col..llen).rev() {
            line[i + 1] = line[i];
        }
        line[ed.cursor_col] = ch;
        line[llen + 1] = 0;
        ed.cursor_col += 1;
        ed.modified = true;
    }
}

fn delete_char(ed: &mut Editor) {
    if !ed.active || ed.cursor_line >= ed.num_lines {
        return;
    }
    if ed.cursor_col > 0 {
        let line = &mut ed.lines[ed.cursor_line];
        let llen = line_len(line);
        for i in (ed.cursor_col - 1)..llen {
            line[i] = line[i + 1];
        }
        ed.cursor_col -= 1;
        ed.modified = true;
    } else if ed.cursor_line > 0 {
        let prev_len = line_len(&ed.lines[ed.cursor_line - 1]);
        let curr_len = line_len(&ed.lines[ed.cursor_line]);
        if prev_len + curr_len < MAX_LINE_LENGTH {
            for i in 0..curr_len {
                ed.lines[ed.cursor_line - 1][prev_len + i] = ed.lines[ed.cursor_line][i];
            }
            ed.lines[ed.cursor_line - 1][prev_len + curr_len] = 0;
            for i in ed.cursor_line..(ed.num_lines - 1) {
                ed.lines[i] = ed.lines[i + 1];
            }
            ed.num_lines -= 1;
            ed.cursor_line -= 1;
            ed.cursor_col = prev_len;
            ed.modified = true;
        }
    }
}

fn new_line(ed: &mut Editor) {
    if !ed.active || ed.num_lines >= MAX_LINES {
        return;
    }
    for i in (ed.cursor_line + 1..ed.num_lines).rev() {
        ed.lines[i + 1] = ed.lines[i];
    }
    let llen = line_len(&ed.lines[ed.cursor_line]);
    if ed.cursor_col < llen {
        let rest = llen - ed.cursor_col;
        for i in 0..rest {
            ed.lines[ed.cursor_line + 1][i] = ed.lines[ed.cursor_line][ed.cursor_col + i];
        }
        ed.lines[ed.cursor_line + 1][rest] = 0;
        ed.lines[ed.cursor_line][ed.cursor_col] = 0;
    } else {
        ed.lines[ed.cursor_line + 1][0] = 0;
    }
    ed.num_lines += 1;
    ed.cursor_line += 1;
    ed.cursor_col = 0;
    ed.modified = true;
}

fn render(ed: &mut Editor) {
    if !ed.active {
        return;
    }
    /* Keep the cursor inside the 20-line window. Every edit/movement path ends
     * in render(), so this one clamp covers them all. */
    if ed.cursor_line < ed.scroll_offset {
        ed.scroll_offset = ed.cursor_line;
    } else if ed.cursor_line >= ed.scroll_offset + EDITOR_DISPLAY_LINES {
        ed.scroll_offset = ed.cursor_line + 1 - EDITOR_DISPLAY_LINES;
    }
    let mut cols = console_ffi::cols();
    let rows = console_ffi::rows();
    /* Clear every row a display line can land on (4..4+20), stopping above the
     * status row (24). rows-2 alone left row 23 stale on a 25-row VGA console. */
    let y_end = (4 + EDITOR_DISPLAY_LINES as u32)
        .min(rows.saturating_sub(1))
        .min(24);
    if cols > 80 {
        cols = 80;
    }
    for y in 4..y_end {
        console_ffi::set_cursor(0, y);
        for _ in 0..cols {
            console_ffi::putchar(' ');
        }
    }

    let mut i = 0usize;
    while i < EDITOR_DISPLAY_LINES && ed.scroll_offset + i < ed.num_lines {
        let line_num = ed.scroll_offset + i;
        let display_y = 4 + i as u32;
        if line_num < ed.num_lines {
            console_ffi::set_cursor(0, display_y);
            print_u32((line_num + 1) as u32, COLOR_INFO);
            print(": ");
            let llen = line_len(&ed.lines[line_num]);
            for j in 0..=llen {
                if line_num == ed.cursor_line && j == ed.cursor_col {
                    print_color("_", COLOR_SUCCESS);
                }
                if j < llen {
                    console_ffi::putchar(ed.lines[line_num][j] as char);
                }
            }
        }
        i += 1;
    }

    console_ffi::set_cursor(0, 24);
    print_color("Line ", COLOR_INFO);
    print_u32((ed.cursor_line + 1) as u32, COLOR_SUCCESS);
    print("/");
    print_u32(ed.num_lines as u32, COLOR_SUCCESS);
    print(" Col:");
    print_u32(ed.cursor_col as u32, COLOR_WHITE);
    if ed.modified {
        print_color(" [Modified]", COLOR_WARNING);
    }
    print(" | ESC for commands (w,q,wq,q!)");
}

fn clamp_col(ed: &mut Editor) {
    let llen = line_len(&ed.lines[ed.cursor_line]);
    if ed.cursor_col > llen {
        ed.cursor_col = llen;
    }
}

fn handle_command_mode(ed: &mut Editor, save_buf: &mut [u8; SAVE_BUF_LEN]) {
    if unsafe { timer_is_poll_mode() } {
        save_inner(ed, save_buf, true);
        if !ed.modified {
            close_clean(ed);
        }
        return;
    }

    console_ffi::set_cursor(0, 23);
    for _ in 0..80 {
        console_ffi::putchar(' ');
    }
    console_ffi::set_cursor(0, 23);
    print_color(":", COLOR_PROMPT);

    let mut cmd_buffer = [0u8; 64];
    let mut cmd_index = 0usize;
    let mut cmd_shift = false;

    loop {
        let mut release_key = 0u8;
        if unsafe { key_queue_pop(&mut release_key) } {
            if release_key == KEY_ESC_RELEASE {
                break;
            }
        } else {
            unsafe {
                core::arch::asm!("sti; hlt", options(nostack, preserves_flags));
            }
        }
    }

    loop {
        let mut cmd_key = 0u8;
        if !unsafe { key_queue_pop(&mut cmd_key) } {
            unsafe {
                core::arch::asm!("sti; hlt", options(nostack, preserves_flags));
            }
            continue;
        }
        if cmd_key & 0x80 != 0 {
            let mk = cmd_key & 0x7F;
            if mk == KBD_SCAN_LSHIFT || mk == KBD_SCAN_RSHIFT {
                cmd_shift = false;
            }
            continue;
        }
        if cmd_key == KBD_SCAN_LSHIFT || cmd_key == KBD_SCAN_RSHIFT {
            cmd_shift = true;
            continue;
        }
        if cmd_key == KEY_ENTER {
            cmd_buffer[cmd_index] = 0;
            console_ffi::set_cursor(0, 22);
            print("Executing: [");
            print(core::str::from_utf8(&cmd_buffer[..cmd_index]).unwrap_or(""));
            print("]");

            if cstr_eq(&cmd_buffer, b"q") {
                if !ed.modified {
                    close_clean(ed);
                } else {
                    console_ffi::set_cursor(0, 21);
                    console_ffi::warning(
                        "Unsaved changes! Use 'q!' to force or 'wq' to save & quit",
                    );
                }
                return;
            }
            if cstr_eq(&cmd_buffer, b"q!") || cstr_eq(&cmd_buffer, b"quit") {
                ed.active = false;
                restore_shell("Dolphin closed (changes discarded)", true);
                return;
            }
            if cstr_eq(&cmd_buffer, b"w") {
                save_inner(ed, save_buf, true);
                return;
            }
            if cstr_eq(&cmd_buffer, b"wq") || cstr_eq(&cmd_buffer, b"x") {
                save_inner(ed, save_buf, true);
                if !ed.modified {
                    close_clean(ed);
                }
                return;
            }
            if cmd_index == 0 {
                render(ed);
                return;
            }
            console_ffi::set_cursor(0, 21);
            console_ffi::error("Unknown cmd. Use: w (save), q (quit), wq (save & quit), q!");
            return;
        }
        if cmd_key == KEY_ESC {
            render(ed);
            return;
        }
        if cmd_key == KEY_BACKSPACE && cmd_index > 0 {
            cmd_index -= 1;
            console_ffi::backspace();
            continue;
        }
        let ch = unsafe { kbd_scancode_to_char(cmd_key, if cmd_shift { 1 } else { 0 }, 0) } as u8;
        if ch >= b' ' && ch < 127 && cmd_index < 63 {
            cmd_buffer[cmd_index] = ch;
            cmd_index += 1;
            cmd_buffer[cmd_index] = 0;
            console_ffi::putchar(ch as char);
        }
    }
}

/// The one save body (`dol -save` from the shell and ESC w / wq / x inside the
/// editor). Caller holds the editor lock (it lends `content`, the save buffer).
/// `in_editor` drops the shell-command tips, which would draw over the editor's
/// status rows.
fn save_inner(ed: &mut Editor, content: &mut [u8; SAVE_BUF_LEN], in_editor: bool) {
    if ed.num_lines == 0 || (ed.num_lines == 1 && ed.lines[0][0] == 0) {
        console_ffi::set_cursor(0, 22);
        print_color("Saving empty file...", COLOR_INFO);
    }
    let mut pos = 0usize;
    let mut truncated = false;
    'lines: for i in 0..ed.num_lines {
        let llen = line_len(&ed.lines[i]);
        for j in 0..llen {
            if pos >= SAVE_BUF_LEN - 1 {
                truncated = true;
                break 'lines;
            }
            content[pos] = ed.lines[i][j];
            pos += 1;
        }
        if i + 1 < ed.num_lines {
            if pos >= SAVE_BUF_LEN - 1 {
                truncated = true;
                break;
            }
            content[pos] = b'\n';
            pos += 1;
        }
    }
    content[pos] = 0;
    let ok = unsafe {
        write_file(
            ed.filename.as_ptr() as *const c_char,
            content.as_ptr() as *const c_char,
        )
    };
    console_ffi::set_cursor(0, 22);
    if ok {
        /* A cut-short save is not a save: keep the buffer marked modified so
         * q / wq / dol -close refuse to throw the rest away. */
        ed.modified = truncated;
        print_color("Saved: ", COLOR_SUCCESS);
        print_color(filename_str(ed), COLOR_WHITE);
        print(" (");
        print_u32(pos as u32, COLOR_WHITE);
        print(" bytes)");
        if truncated {
            print_color(" [TRUNCATED - not all text saved]", COLOR_WARNING);
        }
        return;
    }
    let err = unsafe { get_last_filesystem_error() };
    match err {
        9 => {
            console_ffi::error("Save failed: no Popcorn FAT volume on selected disk");
            if !in_editor {
                println_color(
                    "Tip: disk -list → disk -wipe usb0 YES  (then dol -save)",
                    COLOR_INFO,
                );
            }
        }
        8 | 2 => console_ffi::error("Save failed: bad name (use 8.3 like NOTE.TXT)"),
        6 => console_ffi::error("Save failed: no space / content too large"),
        _ => {
            console_ffi::error("Save failed (disk locked or I/O error)");
            if !in_editor {
                println_color(
                    "Tip: disk -wipe <usbN> YES, then disk -use <usbN>",
                    COLOR_INFO,
                );
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn dolphin_handle_key(keycode: u8) {
    if keycode & 0x80 != 0 {
        return;
    }
    with_editor_save(|ed, save_buf| {
        if !ed.active {
            return;
        }
        match keycode {
            KEY_UP => {
                if ed.cursor_line > 0 {
                    ed.cursor_line -= 1;
                    clamp_col(ed);
                    render(ed);
                }
            }
            KEY_DOWN => {
                if ed.cursor_line + 1 < ed.num_lines {
                    ed.cursor_line += 1;
                    clamp_col(ed);
                    render(ed);
                }
            }
            KEY_LEFT => {
                if ed.cursor_col > 0 {
                    ed.cursor_col -= 1;
                    render(ed);
                } else if ed.cursor_line > 0 {
                    ed.cursor_line -= 1;
                    ed.cursor_col = line_len(&ed.lines[ed.cursor_line]);
                    render(ed);
                }
            }
            KEY_RIGHT => {
                let llen = line_len(&ed.lines[ed.cursor_line]);
                if ed.cursor_col < llen {
                    ed.cursor_col += 1;
                    render(ed);
                } else if ed.cursor_line + 1 < ed.num_lines {
                    ed.cursor_line += 1;
                    ed.cursor_col = 0;
                    render(ed);
                }
            }
            KEY_ENTER => {
                new_line(ed);
                render(ed);
            }
            KEY_BACKSPACE => {
                delete_char(ed);
                render(ed);
            }
            KEY_ESC => handle_command_mode(ed, save_buf),
            _ => {
                let ch = unsafe {
                    kbd_scancode_to_char(keycode, kbd_shift_active(), kbd_caps_active())
                } as u8;
                if ch >= b' ' && ch < 127 {
                    insert_char(ed, ch);
                    render(ed);
                }
            }
        }
    });
}

#[no_mangle]
pub extern "C" fn dolphin_insert_char(ch: u8) {
    with_editor(|ed| {
        insert_char(ed, ch);
    });
}

#[no_mangle]
pub extern "C" fn dolphin_render() {
    with_editor(|ed| render(ed));
}

#[no_mangle]
pub extern "C" fn dolphin_pop_func(_start_pos: u32) {}

#[no_mangle]
pub static dolphin_module: PopModule = PopModule {
    name: NAME.as_ptr() as *const c_char,
    message: MESSAGE.as_ptr() as *const c_char,
    pop_function: Some(dolphin_pop_func),
};
