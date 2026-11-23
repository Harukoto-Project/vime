use std::{
    fmt::Write as _,
    io,
    ptr::{null, null_mut},
    slice,
};

use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};

pub fn protect(key: &str, scope: &str) -> io::Result<String> {
    if key.is_empty() || key.chars().any(char::is_control) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let encrypted = transform(key.as_bytes(), scope, true)?;
    let mut encoded = String::with_capacity(encrypted.len() * 2);
    for byte in encrypted {
        write!(encoded, "{byte:02x}").expect("writing to a String succeeds");
    }
    Ok(encoded)
}

pub fn unprotect(encoded: &str, scope: &str) -> io::Result<String> {
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let encrypted = pairs
        .iter()
        .map(|pair| {
            let digits = str::from_utf8(pair).ok()?;
            u8::from_str_radix(digits, 16).ok()
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(io::ErrorKind::InvalidData)?;
    String::from_utf8(transform(&encrypted, scope, false)?)
        .map_err(|_| io::ErrorKind::InvalidData.into())
}

fn transform(input: &[u8], scope: &str, encrypt: bool) -> io::Result<Vec<u8>> {
    let blob = |data: &[u8]| -> io::Result<CRYPT_INTEGER_BLOB> {
        Ok(CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data.len()).map_err(|_| io::ErrorKind::InvalidInput)?,
            pbData: data.as_ptr().cast_mut(),
        })
    };
    let input = blob(input)?;
    let entropy = blob(scope.as_bytes())?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    // DPAPI borrows both inputs and allocates the output with LocalAlloc.
    unsafe {
        let flags = CRYPTPROTECT_UI_FORBIDDEN;
        let success = if encrypt {
            CryptProtectData(&input, null(), &entropy, null(), null(), flags, &mut output)
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                &entropy,
                null(),
                null(),
                flags,
                &mut output,
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        let data = slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_the_key_and_binds_it_to_its_endpoint() {
        let key = "fake-api-key-for-tests";
        let scope = "test:d1:http://127.0.0.1:8080/v1/systemone";
        let protected = protect(key, scope).unwrap();
        assert!(!protected.contains(key));
        assert_eq!(unprotect(&protected, scope).unwrap(), key);
        assert!(unprotect(&protected, "other endpoint").is_err());
    }
}
