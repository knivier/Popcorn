//! Small shared helpers. `csum` is pure Rust (host-testable); `copy` uses x86 string ops.
pub mod copy;
pub mod csum;
