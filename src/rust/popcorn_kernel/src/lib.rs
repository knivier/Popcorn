#![no_std]

// C console helper (COLOR_LIGHT_GREEN = 0x0A).
extern "C" {
    fn console_println_color(s: *const u8, color: u8);
    fn boot_serial_putc(c: u8);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// Called from C after IDT/PIC and before C pop registration.
#[no_mangle]
pub extern "C" fn rust_init() {
    const MSG: &[u8] = b"Rust active\0";
    const COLOR_LIGHT_GREEN: u8 = 0x0A;
    // Safety: MSG is a NUL-terminated static string; console is initialized by C.
    unsafe {
        console_println_color(MSG.as_ptr(), COLOR_LIGHT_GREEN);
        // Debugcon/serial boot tag so smoke can prove Rust ran ('r').
        boot_serial_putc(b'r');
    }
}
