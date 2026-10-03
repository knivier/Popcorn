pub mod io;
pub mod registry;
pub mod backends;
pub mod bus;

pub use registry::{drive_cmd, init_drive, init_drives, list_devices, list_drives};
pub use registry::{device_ioctl, device_read, device_write};
pub use backends::kbd;
pub use backends::screen::{
    cells_ptr, init_fb, init_vga, screen_backend, screen_clear, screen_cols, screen_fill_panel,
    screen_invalidate, screen_mark_row, screen_paint_bg, screen_present, screen_relayout,
    screen_rows, screen_set_cursor, screen_set_cursor_visible, screen_sync_begin, screen_sync_end,
    screen_write_cell,
};
