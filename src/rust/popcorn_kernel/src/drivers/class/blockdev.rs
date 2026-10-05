//! Block device ops — sectors. Used by ramdisk / virtio-blk (Phase 4).

pub type BlockReadFn = fn(lba: u64, buf: &mut [u8]) -> i64;
pub type BlockWriteFn = fn(lba: u64, buf: &[u8]) -> i64;
pub type BlockIoctlFn = fn(request: u64, argp: *mut u8) -> i64;

pub struct BlockOps {
    pub sector_size: u32,
    pub read: BlockReadFn,
    pub write: BlockWriteFn,
    pub ioctl: BlockIoctlFn,
}

impl BlockOps {
    pub const fn new(
        sector_size: u32,
        read: BlockReadFn,
        write: BlockWriteFn,
        ioctl: BlockIoctlFn,
    ) -> Self {
        Self {
            sector_size,
            read,
            write,
            ioctl,
        }
    }
}

/// Placeholder: no block devices registered yet.
pub fn block_unsupported(_lba: u64, _buf: &mut [u8]) -> i64 {
    -2
}
