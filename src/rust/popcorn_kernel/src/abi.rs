//! C ABI mirrors for Popcorn kernel structs (keep in sync with `src/includes/`).

use core::ffi::c_char;

/// Matches `PopModule` in `src/includes/pop_module.h`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PopModule {
    pub name: *const c_char,
    pub message: *const c_char,
    pub pop_function: Option<extern "C" fn(u32)>,
}

// SAFETY: immortal statics with C-string literals only.
unsafe impl Sync for PopModule {}

/// Matches catalog entry published to C (`src/includes/catalog.h`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CatalogEntryC {
    pub kind: u8,
    pub state: u8,
    pub reserved: u16,
    pub id: u32,
    pub name: [u8; 32],
    pub class_name: [u8; 16],
}

pub const CATALOG_KIND_DRIVE: u8 = 1;
pub const CATALOG_KIND_DEVICE: u8 = 2;
pub const CATALOG_KIND_POP: u8 = 3;
pub const CATALOG_KIND_IRQ: u8 = 4;
pub const CATALOG_KIND_SYSCALL: u8 = 5;
pub const CATALOG_KIND_DISK: u8 = 6;

pub const CATALOG_STATE_IDLE: u8 = 0;
pub const CATALOG_STATE_READY: u8 = 1;
pub const CATALOG_STATE_BOUND: u8 = 2;

/// Ioctl class bytes from `src/includes/ioctl.h`.
pub mod ioctl {
    pub const CLASS_CHAR: u64 = 0x01;
    pub const CLASS_FB: u64 = 0x02;
    pub const CLASS_MEM: u64 = 0x03;
    pub const CLASS_CPU: u64 = 0x04;
    pub const CLASS_CLK: u64 = 0x05;

    pub const fn ioc(class: u64, nr: u64) -> u64 {
        (class << 8) | (nr & 0xFF)
    }
}
