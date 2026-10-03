//! Framebuffer / cell-screen ops (GOP blit + VGA cell write).

pub type FbWriteCellFn = fn(x: u32, y: u32, ch: u8, attr: u8);
pub type FbPresentFn = fn();
pub type FbClearFn = fn(attr: u8);
pub type FbIoctlFn = fn(request: u64, argp: *mut u8) -> i64;

pub struct FbOps {
    pub write_cell: FbWriteCellFn,
    pub present: FbPresentFn,
    pub clear: FbClearFn,
    pub ioctl: FbIoctlFn,
}

impl FbOps {
    pub const fn new(
        write_cell: FbWriteCellFn,
        present: FbPresentFn,
        clear: FbClearFn,
        ioctl: FbIoctlFn,
    ) -> Self {
        Self {
            write_cell,
            present,
            clear,
            ioctl,
        }
    }
}
