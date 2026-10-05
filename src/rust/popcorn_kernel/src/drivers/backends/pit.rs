//! PIT clock backend — thin alias over `clock` until IRQ0 ownership moves here.

#[allow(unused_imports)]
pub use super::clock::{cmd, ioctl, print_summary, probe, read, write};

/// Future: program channel 0 divisor + register IRQ0 handler from Rust.
pub fn pit_hz() -> u32 {
    1000
}
