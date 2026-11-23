use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCryptCreateHash, BCryptDestroyHash,
    BCryptFinishHash, BCryptHashData,
};

use crate::{i18n::tr, system_one::Client};
#[derive(Default)]
pub(crate) struct Progress {
    pub(crate) received: AtomicU64,
    pub(crate) cancelled: AtomicBool,
}

impl Progress {
    pub(crate) fn check(&self) -> io::Result<()> {
        if self.cancelled.load(Relaxed) {
            Err(io::ErrorKind::Interrupted.into())
        } else {
            Ok(())
        }
    }
}

pub(crate) fn fetch(
    url: &str,
    bytes: u64,
    expected: &str,
    directory: &Path,
    progress: &Progress,
) -> io::Result<tempfile::NamedTempFile> {
    progress.check()?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    let mut hash = Sha256::new()?;
    let mut written = 0;
    Client::download(url, |chunk| {
        progress.check()?;
        written += chunk.len() as u64;
        if written > bytes {
            return Err(io::Error::other(tr(
                "ダウンロードしたファイルを検証できませんでした",
            )));
        }
        file.write_all(chunk)?;
        hash.update(chunk)?;
        progress.received.fetch_add(chunk.len() as u64, Relaxed);
        Ok(())
    })?;
    progress.check()?;
    if written != bytes || !hash.matches(expected)? {
        return Err(io::Error::other(tr(
            "ダウンロードしたファイルを検証できませんでした",
        )));
    }
    file.as_file().sync_all()?;
    Ok(file)
}

pub(crate) fn file_sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new()?;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(hash
                .finish()?
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect());
        }
        hash.update(&buffer[..read])?;
    }
}

struct Sha256(BCRYPT_HASH_HANDLE);

impl Sha256 {
    fn new() -> io::Result<Self> {
        let mut handle = null_mut();
        check_hash(unsafe {
            BCryptCreateHash(
                BCRYPT_SHA256_ALG_HANDLE,
                &mut handle,
                null_mut(),
                0,
                null(),
                0,
                0,
            )
        })?;
        Ok(Self(handle))
    }

    fn update(&mut self, bytes: &[u8]) -> io::Result<()> {
        check_hash(unsafe { BCryptHashData(self.0, bytes.as_ptr(), bytes.len() as u32, 0) })
    }

    fn finish(self) -> io::Result<[u8; 32]> {
        let mut digest = [0; 32];
        check_hash(unsafe {
            BCryptFinishHash(self.0, digest.as_mut_ptr(), digest.len() as u32, 0)
        })?;
        Ok(digest)
    }

    fn matches(self, expected: &str) -> io::Result<bool> {
        let digest = self.finish()?;
        let (expected, remainder) = expected.as_bytes().as_chunks::<2>();
        Ok(expected.len() == digest.len()
            && remainder.is_empty()
            && digest.iter().zip(expected).all(|(byte, hex)| {
                let digits = b"0123456789abcdef";
                *hex == [digits[(byte >> 4) as usize], digits[(byte & 0x0f) as usize]]
            }))
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        unsafe { BCryptDestroyHash(self.0) };
    }
}

fn check_hash(status: i32) -> io::Result<()> {
    if status >= 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "BCrypt SHA-256 failed ({status:#x})"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_incremental_download_chunks() {
        let mut hash = Sha256::new().unwrap();
        hash.update(b"a").unwrap();
        hash.update(b"bc").unwrap();
        assert!(
            hash.matches("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
                .unwrap()
        );
    }
}
