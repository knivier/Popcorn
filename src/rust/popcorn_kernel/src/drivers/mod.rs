pub mod io;
pub mod registry;
pub mod backends;
pub mod bus;
pub mod block;
pub mod device;
pub mod driver;
pub mod irq;
pub mod dma;
pub mod usb;
pub mod class;
// Driver framework (virtio-net review, M0). Much of the API is first used by
// virtio-net in M1, so dead-code warnings are silenced for these modules only.
#[allow(dead_code)]
pub mod grant;
#[allow(dead_code)]
pub mod queue;
#[allow(dead_code)]
pub mod virtio;
#[allow(dead_code)]
pub mod net;
pub mod pci_drivers;
pub mod delay;
pub mod wifi;

pub use registry::{drive_cmd, init_drive, init_drives, list_devices, list_drives};
pub use registry::{device_ioctl, device_read, device_write};
pub use backends::kbd;
pub use backends::screen::{
    cells_ptr, init_fb, init_vga, screen_backend, screen_clear, screen_cols, screen_fill_panel,
    screen_invalidate, screen_mark_row, screen_paint_bg, screen_present, screen_relayout,
    screen_rows, screen_set_cursor, screen_set_cursor_visible, screen_sync_begin, screen_sync_end,
    screen_write_cell, screen_enable_shadow, screen_scroll_up, screen_idle_flush,
};
