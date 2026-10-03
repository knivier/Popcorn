//! Character device ops table (byte stream).

pub type CharReadFn = fn(buf: &mut [u8]) -> i64;
pub type CharWriteFn = fn(buf: &[u8]) -> i64;
pub type CharIoctlFn = fn(request: u64, argp: *mut u8) -> i64;

pub struct CharOps {
    pub read: CharReadFn,
    pub write: CharWriteFn,
    pub ioctl: CharIoctlFn,
}

impl CharOps {
    pub const fn new(read: CharReadFn, write: CharWriteFn, ioctl: CharIoctlFn) -> Self {
        Self { read, write, ioctl }
    }
}
