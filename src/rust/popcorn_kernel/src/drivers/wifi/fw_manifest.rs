//! Firmware manifest on the boot stick (`/FW/<chip>/MANIFEST.TXT`, written by
//! scripts/lib/firmware.sh from the pinned scripts/firmware/*.manifest):
//! one line per file, "<sha256 hex> <bytes> <8.3 name>", '#' starts a comment.
//! Pure `core`: host-tested.

use crate::util::sha256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub sha256: [u8; 32],
    pub size: usize,
}

/// The entry for `name` (exact, case-sensitive), or why it is unusable.
pub fn entry(manifest: &[u8], name: &str) -> Result<Entry, &'static str> {
    let text = core::str::from_utf8(manifest).map_err(|_| "manifest is not text")?;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_ascii_whitespace();
        let (Some(sha), Some(size), Some(n), None) = (it.next(), it.next(), it.next(), it.next()) else {
            return Err("malformed manifest line");
        };
        if n != name {
            continue;
        }
        let sha256 = sha256::from_hex(sha.as_bytes()).ok_or("bad SHA-256 in manifest")?;
        let size: usize = size.parse().map_err(|_| "bad size in manifest")?;
        return Ok(Entry { sha256, size });
    }
    Err("file not listed in the manifest")
}

/// Every entry, in file order. Errors on the first malformed line.
pub fn for_each(manifest: &[u8], mut f: impl FnMut(&str, Entry)) -> Result<usize, &'static str> {
    let text = core::str::from_utf8(manifest).map_err(|_| "manifest is not text")?;
    let mut n = 0;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_ascii_whitespace();
        let (Some(sha), Some(size), Some(name), None) = (it.next(), it.next(), it.next(), it.next()) else {
            return Err("malformed manifest line");
        };
        let sha256 = sha256::from_hex(sha.as_bytes()).ok_or("bad SHA-256 in manifest")?;
        let size: usize = size.parse().map_err(|_| "bad size in manifest")?;
        f(name, Entry { sha256, size });
        n += 1;
    }
    Ok(n)
}

/// Ok when `data` is exactly what the manifest promises.
pub fn verify(data: &[u8], e: &Entry) -> Result<(), &'static str> {
    if data.len() != e.size {
        return Err("size differs from the manifest");
    }
    if sha256::digest(data) != e.sha256 {
        return Err("SHA-256 differs from the manifest");
    }
    Ok(())
}
