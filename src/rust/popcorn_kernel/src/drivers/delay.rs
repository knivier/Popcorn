//! Busy-wait delays for driver bring-up sequences (register resets that need
//! "wait 10 ms"). TSC-based once the TSC frequency is known; before that, the
//! xHCI port-0x80 delay.

pub fn delay_ms(ms: u32) {
    let hz = crate::drivers::backends::cpuinfo::tsc_hz();
    if hz == 0 {
        crate::drivers::usb::xhci::delay_ms(ms);
        return;
    }
    let ticks = hz / 1000 * ms as u64;
    // Safety: rdtsc has no side effects.
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    while unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

/// Poll `done` every ~100 µs for up to `ms`. Returns the elapsed ms when it held,
/// None on timeout.
pub fn poll_ms(ms: u32, mut done: impl FnMut() -> bool) -> Option<u32> {
    let hz = crate::drivers::backends::cpuinfo::tsc_hz();
    if hz == 0 {
        for i in 0..ms {
            if done() {
                return Some(i);
            }
            delay_ms(1);
        }
        return if done() { Some(ms) } else { None };
    }
    let per_ms = hz / 1000;
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    loop {
        let el = unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) / per_ms.max(1);
        if done() {
            return Some(el as u32);
        }
        if el >= ms as u64 {
            return None;
        }
        for _ in 0..(per_ms / 10_000).max(1) {
            core::hint::spin_loop();
        }
    }
}
