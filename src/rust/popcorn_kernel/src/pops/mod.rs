//! Pop modules implemented in Rust (same C `PopModule` ABI).

mod registry;
mod shimjapii;
mod spinner;
mod uptime;
mod memory;
mod cpu;
mod sysinfo;

use registry::pop_register_all;

/// C entry: register Rust-owned pops into the C pop table.
#[no_mangle]
pub extern "C" fn rust_pops_register() {
    pop_register_all();
}
