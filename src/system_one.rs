use std::{
    ffi::c_void,
    io,
    mem::size_of,
    ptr::{null, null_mut},
    slice,
    time::{Duration, Instant},
};

use serde_json::Value;
use windows_sys::{Win32::Networking::WinHttp::*, core::w};

mod cancellable;
pub use cancellable::CancellableClient;

struct Endpoint {
    host: Vec<u16>,
    resource: Vec<u16>,
    port: u16,
    secure: bool,
}

impl Endpoint {
    fn parse(url: &str) -> Option<Self> {
        if url.len() > 2048
            || url.contains(['#', '\\'])
            || url.chars().any(char::is_whitespace)
            || url.chars().any(char::is_control)
        {
            return None;
        }
        let input: Vec<u16> = url.encode_utf16().collect();
        // Nonzero lengths with null buffers ask WinHTTP for slices of input.
        let mut parts = URL_COMPONENTS {
            dwStructSize: size_of::<URL_COMPONENTS>() as u32,
            dwHostNameLength: u32::MAX,
            dwUserNameLength: u32::MAX,
            dwPasswordLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            ..Default::default()
        };
        if unsafe { WinHttpCrackUrl(input.as_ptr(), input.len() as u32, 0, &mut parts) } == 0
            || parts.dwHostNameLength == 0
            || parts.dwUserNameLength != 0
            || parts.dwPasswordLength != 0
            || parts.nPort == 0
        {
            return None;
        }
        let secure = match parts.nScheme {
            WINHTTP_INTERNET_SCHEME_HTTP => false,
            WINHTTP_INTERNET_SCHEME_HTTPS => true,
            _ => return None,
        };
        let component = |pointer: *const u16, length: u32| {
            if length == 0 {
                Vec::new()
            } else {
                // WinHTTP points into the still-live input buffer.
                unsafe { slice::from_raw_parts(pointer, length as usize) }.to_vec()
            }
        };
        let mut host = component(parts.lpszHostName, parts.dwHostNameLength);
        let mut resource = component(parts.lpszUrlPath, parts.dwUrlPathLength);
        if resource.is_empty() {
            resource.push(b'/' as u16);
        }
        resource.extend(component(parts.lpszExtraInfo, parts.dwExtraInfoLength));
        host.push(0);
        resource.push(0);
        Some(Self {
            host,
            resource,
            port: parts.nPort,
            secure,
        })
    }
}

pub fn valid_url(url: &str) -> bool {
    Endpoint::parse(url).is_some()
}

struct Handle(*mut c_void);

impl Handle {
    fn new(handle: *mut c_void) -> io::Result<Self> {
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

pub struct Client(Handle);

impl Client {
    pub fn new() -> io::Result<Self> {
        Self::open(WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, 3500)
    }

    pub fn loopback() -> io::Result<Self> {
        Self::open(WINHTTP_ACCESS_TYPE_NO_PROXY, 15_000)
    }

    fn open(proxy: u32, receive_timeout: i32) -> io::Result<Self> {
        let session = Handle::new(unsafe { WinHttpOpen(w!("ViME"), proxy, null(), null(), 0) })?;
        checked(unsafe { WinHttpSetTimeouts(session.0, 1500, 1500, 1500, receive_timeout) })?;
        Ok(Self(session))
    }

    pub fn post(&self, url: &str, key: &str, body: &Value) -> io::Result<Value> {
        let endpoint = Endpoint::parse(url).ok_or(io::ErrorKind::InvalidInput)?;
        if key.chars().any(char::is_control) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut headers = String::from("Content-Type: application/json; charset=utf-8\r\n");
        if !key.is_empty() {
            headers.push_str(&format!("Authorization: Bearer {key}\r\n"));
        }
        let headers: Vec<u16> = headers.encode_utf16().chain([0]).collect();
        let body = serde_json::to_vec(body)?;
        let request = self.request(&endpoint, w!("POST"), false)?;
        request.send(&headers, &body)?;
        let started = Instant::now();
        let mut response = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read = request.read(&mut chunk)?;
            if read == 0 {
                return Ok(serde_json::from_slice(&response)?);
            }
            if response.len() + read > 64 * 1024 || started.elapsed() > Duration::from_secs(4) {
                return Err(io::ErrorKind::InvalidData.into());
            }
            response.extend_from_slice(&chunk[..read]);
        }
    }

    /// Stream an HTTPS download, including CDN redirects, without buffering it in memory.
    pub fn download(url: &str, mut consume: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        let endpoint = Endpoint::parse(url)
            .filter(|endpoint| endpoint.secure)
            .ok_or(io::ErrorKind::InvalidInput)?;
        let client = Self::open(WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, 20_000)?;
        let request = client.request(&endpoint, w!("GET"), true)?;
        request.send(&[], &[])?;
        let mut chunk = vec![0u8; 256 * 1024];
        loop {
            let read = request.read(&mut chunk)?;
            if read == 0 {
                return Ok(());
            }
            consume(&chunk[..read])?;
        }
    }

    fn request(
        &self,
        endpoint: &Endpoint,
        method: *const u16,
        redirects: bool,
    ) -> io::Result<Request> {
        open_request(&self.0, endpoint, method, redirects)
    }
}

fn open_request(
    session: &Handle,
    endpoint: &Endpoint,
    method: *const u16,
    redirects: bool,
) -> io::Result<Request> {
    unsafe {
        let connection = Handle::new(WinHttpConnect(
            session.0,
            endpoint.host.as_ptr(),
            endpoint.port,
            0,
        ))?;
        let handle = Handle::new(WinHttpOpenRequest(
            connection.0,
            method,
            endpoint.resource.as_ptr(),
            null(),
            null(),
            null(),
            if endpoint.secure {
                WINHTTP_FLAG_SECURE
            } else {
                0
            },
        ))?;
        // POST cannot forward typed text or credentials elsewhere. GET can
        // follow redirects to HTTPS CDNs and never carries credentials.
        let disabled = WINHTTP_DISABLE_COOKIES
            | if redirects {
                0
            } else {
                WINHTTP_DISABLE_REDIRECTS
            };
        checked(WinHttpSetOption(
            handle.0,
            WINHTTP_OPTION_DISABLE_FEATURE,
            (&raw const disabled).cast(),
            size_of::<u32>() as u32,
        ))?;
        if redirects {
            let policy = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
            checked(WinHttpSetOption(
                handle.0,
                WINHTTP_OPTION_REDIRECT_POLICY,
                (&raw const policy).cast(),
                size_of::<u32>() as u32,
            ))?;
        }
        Ok(Request {
            handle,
            _connection: connection,
        })
    }
}

struct Request {
    handle: Handle,
    _connection: Handle,
}

impl Request {
    fn send(&self, headers: &[u16], body: &[u8]) -> io::Result<()> {
        let (header_ptr, header_len) = if headers.is_empty() {
            (null(), 0)
        } else {
            (headers.as_ptr(), u32::MAX)
        };
        let body_ptr = if body.is_empty() {
            null()
        } else {
            body.as_ptr().cast()
        };
        unsafe {
            checked(WinHttpSendRequest(
                self.handle.0,
                header_ptr,
                header_len,
                body_ptr,
                body.len() as u32,
                body.len() as u32,
                0,
            ))?;
            checked(WinHttpReceiveResponse(self.handle.0, null_mut()))?;
        }
        self.require_success()
    }

    fn require_success(&self) -> io::Result<()> {
        let mut status = 0u32;
        checked(unsafe {
            WinHttpQueryHeaders(
                self.handle.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&raw mut status).cast(),
                &mut (size_of::<u32>() as u32),
                null_mut(),
            )
        })?;
        if !(200..300).contains(&status) {
            return Err(io::Error::other(format!("HTTP {status}")));
        }
        Ok(())
    }

    fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut read = 0;
        checked(unsafe {
            WinHttpReadData(
                self.handle.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
        })?;
        Ok(read as usize)
    }
}
fn checked(success: i32) -> io::Result<()> {
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posts_utf8_and_an_optional_bearer_key_over_http() {
        use std::{
            io::{BufRead, BufReader, Read, Write},
            net::TcpListener,
            thread,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            assert_eq!(first, "POST /v1/systemone HTTP/1.1\r\n");
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line.to_ascii_lowercase());
            }
            assert!(headers.contains("authorization: bearer test-key\r\n"));
            assert!(headers.contains("content-type: application/json; charset=utf-8\r\n"));
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let mut payload = vec![0; length];
            reader.read_exact(&mut payload).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&payload).unwrap()["state"],
                "こんにちは"
            );
            let response = r#"{"answers":{"typo":{"choice":"keep"}}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response
            )
            .unwrap();
        });
        let response = Client::new()
            .unwrap()
            .post(
                &url,
                "test-key",
                &serde_json::json!({"state": "こんにちは"}),
            )
            .unwrap();
        assert_eq!(response["answers"]["typo"]["choice"], "keep");
        server.join().unwrap();
    }

    #[test]
    fn parses_official_and_local_urls_without_losing_the_port_or_query() {
        assert!(valid_url("https://api.typesafe.ai/v1/systemone"));
        let endpoint = Endpoint::parse("http://127.0.0.1:8080/v1/systemone?runner=d1").unwrap();
        assert_eq!(endpoint.port, 8080);
        assert!(!endpoint.secure);
        assert_eq!(
            String::from_utf16_lossy(&endpoint.resource),
            "/v1/systemone?runner=d1\0"
        );
        assert!(!valid_url("ftp://example.com/v1/systemone"));
        assert!(!valid_url("https://user:password@example.com/v1/systemone"));
    }
}
