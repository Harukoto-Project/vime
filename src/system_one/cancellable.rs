use std::{
    cell::UnsafeCell,
    ffi::c_void,
    io,
    mem::size_of,
    ptr::{null, null_mut},
    slice,
    sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    time::{Duration, Instant},
};

use super::{Endpoint, Handle, Value, checked, open_request};
use windows_sys::{
    Win32::{Foundation::ERROR_IO_PENDING, Networking::WinHttp::*},
    core::w,
};

const CANCEL_POLL: Duration = Duration::from_millis(15);
const MAX_RESPONSE: usize = 64 * 1024;

pub struct CancellableClient(Handle);

// The shared handle is a WinHTTP session, whose options are fixed at creation.
// Each caller owns its independent connection and request handles.
unsafe impl Send for CancellableClient {}
unsafe impl Sync for CancellableClient {}

impl CancellableClient {
    pub fn new(loopback: bool) -> io::Result<Self> {
        let proxy = if loopback {
            WINHTTP_ACCESS_TYPE_NO_PROXY
        } else {
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY
        };
        let handle = Handle::new(unsafe {
            WinHttpOpen(w!("ViME"), proxy, null(), null(), WINHTTP_FLAG_ASYNC)
        })?;
        Ok(Self(handle))
    }

    pub fn post(
        &self,
        url: &str,
        key: &str,
        body: &Value,
        deadline: Instant,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<Value> {
        self.post_inner(url, key, body, deadline, cancelled, true)
    }

    pub fn post_without_response(
        &self,
        url: &str,
        body: &Value,
        deadline: Instant,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<()> {
        self.post_inner(url, "", body, deadline, cancelled, false)
            .map(|_| ())
    }

    /// Like [`Self::new`], for responses that stream for a long time: a model
    /// may need a while to load before it produces the first byte.
    pub fn streaming(loopback: bool) -> io::Result<Self> {
        let client = Self::new(loopback)?;
        checked(unsafe { WinHttpSetTimeouts(client.0.0, 5_000, 5_000, 30_000, 120_000) })?;
        Ok(client)
    }

    /// Posts JSON and hands the response body to `consume` as it arrives.
    pub fn post_stream(
        &self,
        url: &str,
        key: &str,
        body: &Value,
        deadline: Instant,
        cancelled: impl Fn() -> bool,
        consume: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        self.exchange(url, key, body, deadline, cancelled, true, consume)
    }

    fn post_inner(
        &self,
        url: &str,
        key: &str,
        body: &Value,
        deadline: Instant,
        cancelled: impl Fn() -> bool,
        read_response: bool,
    ) -> io::Result<Value> {
        let mut response = Vec::new();
        self.exchange(
            url,
            key,
            body,
            deadline,
            cancelled,
            read_response,
            |chunk| {
                if response.len() + chunk.len() > MAX_RESPONSE {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                response.extend_from_slice(chunk);
                Ok(())
            },
        )?;
        if read_response {
            Ok(serde_json::from_slice(&response)?)
        } else {
            Ok(Value::Null)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exchange(
        &self,
        url: &str,
        key: &str,
        body: &Value,
        deadline: Instant,
        cancelled: impl Fn() -> bool,
        read_response: bool,
        mut consume: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        valid_time(deadline, &cancelled)?;
        let endpoint = Endpoint::parse(url).ok_or(io::ErrorKind::InvalidInput)?;
        if key.chars().any(char::is_control) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let request = open_request(&self.0, &endpoint, w!("POST"), false)?;
        let mut headers = "Content-Type: application/json; charset=utf-8\r\n".to_owned();
        if !key.is_empty() {
            headers.push_str(&format!("Authorization: Bearer {key}\r\n"));
        }
        let (sender, events) = channel();
        let state = Box::new(CallbackState {
            sender,
            headers: headers.encode_utf16().chain([0]).collect(),
            body: serde_json::to_vec(body)?,
            read: UnsafeCell::new([0; 8192]),
        });
        // The context owns every buffer borrowed by an asynchronous operation.
        // WinHTTP frees it via HANDLE_CLOSING, including on timeout/cancellation.
        let state = Box::into_raw(state);
        let handle = request.handle.0;
        unsafe {
            let cb = WinHttpSetStatusCallback(
                handle,
                Some(on_status),
                WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING
                    | WINHTTP_CALLBACK_STATUS_HANDLE_CREATED
                    | WINHTTP_CALLBACK_FLAG_REQUEST_ERROR
                    | WINHTTP_CALLBACK_FLAG_SENDREQUEST_COMPLETE
                    | WINHTTP_CALLBACK_FLAG_HEADERS_AVAILABLE
                    | WINHTTP_CALLBACK_FLAG_DATA_AVAILABLE
                    | WINHTTP_CALLBACK_FLAG_READ_COMPLETE,
                0,
            );
            if cb.is_some_and(|previous| previous as usize == usize::MAX) {
                drop(Box::from_raw(state));
                return Err(io::Error::last_os_error());
            }
            let context = state as usize;
            if let Err(error) = checked(WinHttpSetOption(
                handle,
                WINHTTP_OPTION_CONTEXT_VALUE,
                (&raw const context).cast(),
                size_of::<usize>() as u32,
            )) {
                drop(Box::from_raw(state));
                return Err(error);
            }
            let buffers = &*state;
            let length = buffers.body.len() as u32;
            valid_time(deadline, &cancelled)?;
            started(WinHttpSendRequest(
                handle,
                buffers.headers.as_ptr(),
                u32::MAX,
                buffers.body.as_ptr().cast(),
                length,
                length,
                context,
            ))?;
            wait(
                &events,
                deadline,
                &cancelled,
                WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE,
            )?;
            started(WinHttpReceiveResponse(handle, null_mut()))?;
            wait(
                &events,
                deadline,
                &cancelled,
                WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE,
            )?;
        }
        request.require_success()?;
        if !read_response {
            return Ok(());
        }
        loop {
            valid_time(deadline, &cancelled)?;
            // A read alone may wait for its whole buffer to fill, which would
            // hold back a streamed answer; asking what is available first
            // returns as soon as any data has arrived.
            unsafe { started(WinHttpQueryDataAvailable(handle, null_mut()))? };
            let available = wait(
                &events,
                deadline,
                &cancelled,
                WINHTTP_CALLBACK_STATUS_DATA_AVAILABLE,
            )?;
            let available = available
                .try_into()
                .map(u32::from_le_bytes)
                .map_err(|_| io::ErrorKind::InvalidData)?;
            if available == 0 {
                return Ok(());
            }
            valid_time(deadline, &cancelled)?;
            unsafe {
                started(WinHttpReadData(
                    handle,
                    (*state).read.get().cast(),
                    available.min(8192),
                    null_mut(),
                ))?;
            }
            let chunk = wait(
                &events,
                deadline,
                &cancelled,
                WINHTTP_CALLBACK_STATUS_READ_COMPLETE,
            )?;
            if chunk.is_empty() {
                return Ok(());
            }
            consume(&chunk)?;
        }
    }
}

struct CallbackState {
    sender: Sender<Event>,
    headers: Vec<u16>,
    body: Vec<u8>,
    read: UnsafeCell<[u8; 8192]>,
}

enum Event {
    Complete(u32, Vec<u8>),
    Error(u32),
}

unsafe extern "system" fn on_status(
    _: *mut c_void,
    context: usize,
    status: u32,
    data: *mut c_void,
    size: u32,
) {
    if context == 0 {
        return;
    }
    let pointer = context as *mut CallbackState;
    if status == WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING {
        unsafe { drop(Box::from_raw(pointer)) };
        return;
    }
    let event = match status {
        WINHTTP_CALLBACK_STATUS_REQUEST_ERROR
            if size as usize >= size_of::<WINHTTP_ASYNC_RESULT>() =>
        {
            Event::Error(unsafe { data.cast::<WINHTTP_ASYNC_RESULT>().read_unaligned() }.dwError)
        }
        // The status information is the number of bytes that can be read.
        WINHTTP_CALLBACK_STATUS_DATA_AVAILABLE if size as usize >= size_of::<u32>() => {
            Event::Complete(
                status,
                unsafe { data.cast::<u32>().read_unaligned() }
                    .to_le_bytes()
                    .to_vec(),
            )
        }
        WINHTTP_CALLBACK_STATUS_READ_COMPLETE if size > 0 => Event::Complete(
            status,
            unsafe { slice::from_raw_parts(data.cast::<u8>(), size as usize) }.to_vec(),
        ),
        WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE
        | WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE
        | WINHTTP_CALLBACK_STATUS_READ_COMPLETE => Event::Complete(status, Vec::new()),
        _ => return,
    };
    let _ = unsafe { &(*pointer).sender }.send(event);
}

fn started(result: i32) -> io::Result<()> {
    checked(result).or_else(|error| {
        if error.raw_os_error() == Some(ERROR_IO_PENDING as i32) {
            Ok(())
        } else {
            Err(error)
        }
    })
}

fn valid_time(deadline: Instant, cancelled: &impl Fn() -> bool) -> io::Result<Duration> {
    if cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::ErrorKind::TimedOut.into());
    }
    Ok(remaining)
}

fn wait(
    events: &Receiver<Event>,
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
    wanted: u32,
) -> io::Result<Vec<u8>> {
    loop {
        let interval = valid_time(deadline, cancelled)?.min(CANCEL_POLL);
        match events.recv_timeout(interval) {
            Ok(Event::Complete(status, bytes)) if status == wanted => {
                valid_time(deadline, cancelled)?;
                return Ok(bytes);
            }
            Ok(Event::Error(code)) => return Err(io::Error::from_raw_os_error(code as i32)),
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::ErrorKind::ConnectionAborted.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicBool, Ordering::Relaxed},
        thread,
    };

    #[test]
    fn posts_a_utf8_report_and_accepts_an_empty_success_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1/typos", listener.local_addr().unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        let payload = serde_json::json!({"original":"異義","replacement":"異議"});
        let client = CancellableClient::new(true).unwrap();
        thread::scope(|scope| {
            let request =
                scope.spawn(|| client.post_without_response(&url, &payload, deadline, || false));
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("mock server did not get a report: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim_end(), "POST /v1/typos HTTP/1.1");
            let mut length = None;
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                let header = line.to_ascii_lowercase();
                if let Some(value) = header.strip_prefix("content-length:") {
                    length = Some(value.trim().parse().unwrap());
                }
                assert!(!header.starts_with("authorization:"));
            }
            let mut body = vec![0; length.unwrap()];
            reader.read_exact(&mut body).unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), payload);
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .unwrap();
            request.join().unwrap().unwrap();
        });
    }

    #[test]
    fn cancellation_closes_a_request_waiting_for_response_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let cancelled = AtomicBool::new(false);
        let client = CancellableClient::new(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        thread::scope(|scope| {
            let request = scope.spawn(|| {
                client.post(
                    &url,
                    "test-key",
                    &serde_json::json!({"state":"synthetic"}),
                    deadline,
                    || cancelled.load(Relaxed),
                )
            });
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("mock server did not get a request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0; 2048];
            assert!(stream.read(&mut buffer).unwrap() > 0);
            let cancelled_at = Instant::now();
            cancelled.store(true, Relaxed);
            assert_eq!(
                request.join().unwrap().unwrap_err().kind(),
                io::ErrorKind::Interrupted
            );
            assert!(cancelled_at.elapsed() < Duration::from_secs(1));
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::ConnectionReset => break,
                    Err(error) => panic!("cancelled request kept its socket open: {error}"),
                }
            }
        });
    }
}
