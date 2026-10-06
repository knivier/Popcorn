//! On-disk filesystem — FAT32 on the selected block disk.
//!
//! C-facing wrappers: every pointer is null-checked, every string is bounded and
//! must be valid UTF-8; bad input sets `get_last_filesystem_error()` and fails
//! instead of being truncated or reinterpreted.

pub mod fat32;

use core::ffi::c_char;
use core::sync::atomic::{AtomicI32, Ordering};

use crate::console_ffi::{println_color, COLOR_LIGHT_CYAN, COLOR_WHITE};

const ERR_SUCCESS: i32 = 0;
const ERR_INVALID_INPUT: i32 = 2;
const ERR_BUFFER_OVERFLOW: i32 = 3;
const ERR_NOT_FOUND: i32 = 4;
const ERR_ALREADY_EXISTS: i32 = 5;
const ERR_NO_SPACE: i32 = 6;
const ERR_NAME_TOO_LONG: i32 = 8;
const ERR_INVALID_OPERATION: i32 = 9;

/// Longest file/dir/path argument accepted from C.
const MAX_NAME: usize = 64;

static LAST_ERR: AtomicI32 = AtomicI32::new(ERR_SUCCESS);

fn set_err(e: i32) {
    LAST_ERR.store(e, Ordering::Release);
}

/// Borrow a NUL-terminated C string of at most `max` bytes (excluding the NUL).
/// `None` if the pointer is null, no NUL appears within `max + 1` bytes, or the
/// bytes are not UTF-8.
fn cstr_n<'a>(p: *const c_char, max: usize) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    let mut len = 0usize;
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
            if len > max {
                return None;
            }
        }
        core::str::from_utf8(core::slice::from_raw_parts(p as *const u8, len)).ok()
    }
}

/// Map a bad C argument to the right error code (overflow vs. plain invalid).
fn bad_arg(p: *const c_char, max: usize) {
    if p.is_null() {
        set_err(ERR_INVALID_INPUT);
        return;
    }
    let mut len = 0usize;
    unsafe {
        while len <= max && *p.add(len) != 0 {
            len += 1;
        }
    }
    set_err(if len > max { ERR_NAME_TOO_LONG } else { ERR_INVALID_INPUT });
}

fn name_arg<'a>(p: *const c_char) -> Option<&'a str> {
    let s = cstr_n(p, MAX_NAME);
    if s.is_none() {
        bad_arg(p, MAX_NAME);
    }
    s
}

fn err_code(e: fat32::FsError, fallback: i32) -> i32 {
    match e {
        fat32::FsError::NotFound => ERR_NOT_FOUND,
        fat32::FsError::Exists => ERR_ALREADY_EXISTS,
        fat32::FsError::NoSpace => ERR_NO_SPACE,
        fat32::FsError::NameTooLong => ERR_NAME_TOO_LONG,
        _ => fallback,
    }
}

/// Mount (or format) FAT32 on the currently selected disk.
pub fn init() {
    if crate::drivers::block::selected_id() == usize::MAX {
        set_err(ERR_INVALID_OPERATION);
        println_color(
            "FAT32: no disk selected (USB MSC not found? try: disk list)",
            COLOR_WHITE,
        );
        return;
    }
    match fat32::mount_or_format() {
        Ok(()) => {
            set_err(ERR_SUCCESS);
            println_color("FAT32 mounted", COLOR_LIGHT_CYAN);
        }
        Err(fat32::FsError::NotFat) => {
            set_err(ERR_INVALID_OPERATION);
            println_color(
                "FAT32: no Popcorn volume (blank/foreign — use: disk wipe <name> YES)",
                COLOR_WHITE,
            );
        }
        Err(fat32::FsError::NoSpace) => {
            set_err(ERR_NO_SPACE);
            println_color(
                "FAT32: disk too small to format (need ~33MiB+)",
                COLOR_WHITE,
            );
        }
        Err(_) => {
            set_err(ERR_INVALID_OPERATION);
            println_color("FAT32 mount failed", COLOR_WHITE);
        }
    }
}

/// Drop the current volume and mount whatever disk is now selected.
/// Called after `disk use` / `disk install` so FAT never keeps pointing at the old disk.
pub fn remount_selected() {
    fat32::unmount();
    init();
}

#[no_mangle]
pub extern "C" fn init_filesystem() {
    init();
}

#[no_mangle]
pub extern "C" fn get_last_filesystem_error() -> i32 {
    LAST_ERR.load(Ordering::Acquire)
}

#[no_mangle]
pub extern "C" fn write_file(name: *const c_char, content: *const c_char) -> bool {
    let Some(n) = name_arg(name) else {
        return false;
    };
    let Some(c) = cstr_n(content, fat32::MAX_CONTENT) else {
        if content.is_null() {
            set_err(ERR_INVALID_INPUT);
        } else {
            set_err(ERR_BUFFER_OVERFLOW);
        }
        return false;
    };
    /* If nothing is mounted yet (e.g. just selected a wiped disk), try once. */
    match fat32::write_file(n, c.as_bytes()) {
        Ok(()) => {
            set_err(ERR_SUCCESS);
            return true;
        }
        Err(fat32::FsError::NotMounted) => {
            remount_selected();
        }
        Err(e) => {
            set_err(err_code(e, ERR_INVALID_OPERATION));
            return false;
        }
    }
    match fat32::write_file(n, c.as_bytes()) {
        Ok(()) => {
            set_err(ERR_SUCCESS);
            true
        }
        Err(e) => {
            set_err(err_code(e, ERR_INVALID_OPERATION));
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn read_file(name: *const c_char) -> *const c_char {
    let Some(n) = name_arg(name) else {
        return core::ptr::null();
    };
    match fat32::read_file(n) {
        Ok(p) => {
            set_err(ERR_SUCCESS);
            p
        }
        Err(e) => {
            set_err(err_code(e, ERR_NOT_FOUND));
            core::ptr::null()
        }
    }
}

#[no_mangle]
pub extern "C" fn list_files() {
    list_files_console();
}

#[no_mangle]
pub extern "C" fn list_files_console() {
    match fat32::list_cwd() {
        Ok(()) => set_err(ERR_SUCCESS),
        Err(_) => set_err(ERR_INVALID_OPERATION),
    }
}

#[no_mangle]
pub extern "C" fn delete_file(name: *const c_char) -> bool {
    let Some(n) = name_arg(name) else {
        return false;
    };
    match fat32::delete_entry(n) {
        Ok(()) => {
            set_err(ERR_SUCCESS);
            true
        }
        Err(e) => {
            set_err(err_code(e, ERR_NOT_FOUND));
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn create_directory(name: *const c_char) -> bool {
    let Some(n) = name_arg(name) else {
        return false;
    };
    match fat32::mkdir(n) {
        Ok(()) => {
            set_err(ERR_SUCCESS);
            true
        }
        Err(e) => {
            set_err(err_code(e, ERR_NO_SPACE));
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn change_directory(name: *const c_char) -> bool {
    let Some(n) = name_arg(name) else {
        return false;
    };
    match fat32::chdir(n) {
        Ok(()) => {
            set_err(ERR_SUCCESS);
            true
        }
        Err(e) => {
            set_err(err_code(e, ERR_INVALID_OPERATION));
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn get_current_directory() -> *const c_char {
    fat32::cwd_cstr()
}

#[no_mangle]
pub extern "C" fn search_file(name: *const c_char) -> *const c_char {
    let Some(n) = name_arg(name) else {
        return core::ptr::null();
    };
    match fat32::search(n) {
        Ok(p) => {
            set_err(ERR_SUCCESS);
            p
        }
        Err(e) => {
            set_err(err_code(e, ERR_NOT_FOUND));
            core::ptr::null()
        }
    }
}

#[no_mangle]
pub extern "C" fn copy_file(src: *const c_char, dest_path: *const c_char) -> bool {
    let Some(s) = name_arg(src) else {
        return false;
    };
    let Some(d) = name_arg(dest_path) else {
        return false;
    };
    match fat32::copy_file(s, d) {
        Ok(()) => {
            set_err(ERR_SUCCESS);
            true
        }
        Err(e) => {
            set_err(err_code(e, ERR_NOT_FOUND));
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn list_hierarchy() {
    match fat32::list_hierarchy() {
        Ok(()) => set_err(ERR_SUCCESS),
        Err(_) => set_err(ERR_INVALID_OPERATION),
    }
}
