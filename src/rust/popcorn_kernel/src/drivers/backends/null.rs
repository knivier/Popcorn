pub fn probe() -> Result<(), &'static str> {
    Ok(())
}

pub fn read(_buf: &mut [u8]) -> i64 {
    0
}

pub fn write(buf: &[u8]) -> i64 {
    buf.len() as i64
}

pub fn ioctl(_request: u64, _argp: *mut u8) -> i64 {
    -2
}
