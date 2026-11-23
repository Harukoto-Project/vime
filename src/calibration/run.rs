use std::{
    io,
    sync::atomic::{AtomicBool, Ordering::Relaxed},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;

use super::{Limits, MAX_CONCURRENCY, MAX_DEBOUNCE_MS, Measurement, now_seconds};
use crate::system_one::CancellableClient;

pub const BUDGET: Duration = Duration::from_secs(58);
const LATENCY_LIMIT: Duration = Duration::from_secs(1);
const REQUEST_BUDGET: Duration = Duration::from_secs(4);
const MIN_THROUGHPUT_GAIN: f64 = 1.05;

struct Stage {
    concurrent: u16,
    p95: Duration,
    throughput: f64,
}

pub fn measure(
    url: &str,
    key: &str,
    model: Option<&str>,
    loopback: bool,
    deadline: Instant,
    cancelled: impl Fn() -> bool + Sync,
) -> io::Result<Measurement> {
    let client = CancellableClient::new(loopback)?;
    let mut request = json!({
        "state": "Synthetic calibration sample. The user typed vritual and corrected it to virtual.",
        "questions": {"typo": {
            "type": "choice",
            "instructions": "Did the user correct an accidental typing error?",
            "criteria": {"yes": "A clear spelling correction", "no": "An intentional change or no error"}
        }}
    });
    if let Some(model) = model {
        request["model"] = json!(model);
    }
    let send = |budget, cancel: &(dyn Fn() -> bool + Sync)| {
        let limit = (Instant::now() + budget).min(deadline);
        let response = client.post(url, key, &request, limit, cancel)?;
        match response["answers"]["typo"]["choice"].as_str() {
            Some("yes" | "no") => Ok(()),
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    };

    // Local GPU engines can compile kernels during their first decision.
    // Give that warm-up more time within the same overall deadline.
    let warmup = if loopback { 25 } else { 12 };
    send(Duration::from_secs(warmup), &cancelled)?;
    let sequential_budget = if loopback { 12 } else { 6 };
    let sequential_send =
        |cancel: &(dyn Fn() -> bool + Sync)| send(Duration::from_secs(sequential_budget), cancel);
    let sequential = stage(1, 4, &sequential_send, &cancelled)?;
    let send = |cancel: &(dyn Fn() -> bool + Sync)| send(REQUEST_BUDGET, cancel);
    let serial_p95 = sequential.p95;
    let mut best = sequential;
    if best.p95 <= LATENCY_LIMIT {
        let mut concurrent = 2;
        while concurrent <= MAX_CONCURRENCY && Instant::now() < deadline && !cancelled() {
            let Ok(result) = stage(concurrent, 2, &send, &cancelled) else {
                break;
            };
            if result.p95 > LATENCY_LIMIT
                || result.throughput <= best.throughput * MIN_THROUGHPUT_GAIN
            {
                break;
            }
            best = result;
            concurrent *= 2;
        }
    }
    if cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let rate = best.throughput * 0.8;
    let debounce = (1000.0 / rate).ceil() as u64;
    Ok(Measurement {
        limits: Limits {
            concurrent: best.concurrent,
            requests_per_second: rate,
            debounce_ms: debounce.max(millis(serial_p95)).clamp(150, MAX_DEBOUNCE_MS),
        },
        measured_at: now_seconds(),
        sequential_p95_ms: millis(serial_p95),
        p95_ms: millis(best.p95),
        throughput: best.throughput,
    })
}

fn stage(
    concurrent: u16,
    samples: usize,
    send: &(impl Fn(&(dyn Fn() -> bool + Sync)) -> io::Result<()> + Sync),
    cancelled: &(impl Fn() -> bool + Sync),
) -> io::Result<Stage> {
    let failed = AtomicBool::new(false);
    let started = Instant::now();
    let mut times = if concurrent == 1 {
        sample(samples, send, cancelled, &failed)?
    } else {
        thread::scope(|scope| {
            let mut workers = Vec::with_capacity(usize::from(concurrent));
            for _ in 0..concurrent {
                let failed = &failed;
                workers.push(
                    thread::Builder::new()
                        .spawn_scoped(scope, move || sample(samples, send, cancelled, failed))?,
                );
            }
            let mut times = Vec::new();
            let mut first = None;
            for worker in workers {
                match worker
                    .join()
                    .map_err(|_| io::Error::other("calibration worker stopped"))?
                {
                    Ok(samples) => times.extend(samples),
                    Err(error) if first.is_none() || error.kind() != io::ErrorKind::Interrupted => {
                        first = Some(error);
                    }
                    Err(_) => {}
                }
            }
            first.map_or(Ok(times), Err)
        })?
    };
    let elapsed = started.elapsed().as_secs_f64();
    times.sort_unstable();
    let rank = (times.len() * 95).div_ceil(100) - 1;
    Ok(Stage {
        concurrent,
        p95: times[rank],
        throughput: times.len() as f64 / elapsed,
    })
}

fn sample(
    count: usize,
    send: &impl Fn(&(dyn Fn() -> bool + Sync)) -> io::Result<()>,
    cancelled: &(impl Fn() -> bool + Sync),
    failed: &AtomicBool,
) -> io::Result<Vec<Duration>> {
    let mut times = Vec::with_capacity(count);
    for _ in 0..count {
        let started = Instant::now();
        if let Err(error) = send(&|| cancelled() || failed.load(Relaxed)) {
            failed.store(true, Relaxed);
            return Err(error);
        }
        times.push(started.elapsed());
    }
    Ok(times)
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::atomic::AtomicUsize,
    };

    #[test]
    fn times_out_a_server_that_does_not_send_a_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let started = Instant::now();
        let error = measure(
            &url,
            "",
            None,
            true,
            started + Duration::from_millis(100),
            || false,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn keeps_the_previous_level_if_parallel_p95_exceeds_one_second() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let stop = AtomicBool::new(false);
        let active = AtomicUsize::new(0);
        let max = AtomicUsize::new(0);
        thread::scope(|scope| {
            let server = scope.spawn(|| {
                thread::scope(|handlers| {
                    while !stop.load(Relaxed) {
                        let mut stream = match listener.accept() {
                            Ok((stream, _)) => stream,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(2));
                                continue;
                            }
                            Err(error) => panic!("{error}"),
                        };
                        let active = &active;
                        let max = &max;
                        handlers.spawn(move || {
                            stream.set_nonblocking(false).unwrap();
                            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                            let mut reader = BufReader::new(&mut stream);
                            let mut length = 0;
                            loop {
                                let mut line = String::new();
                                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" { break; }
                                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length: ") {
                                    length = value.trim().parse::<usize>().unwrap();
                                }
                            }
                            let mut body = vec![0; length];
                            reader.read_exact(&mut body).unwrap();
                            assert!(String::from_utf8_lossy(&body).contains("Synthetic calibration sample"));
                            let concurrent = active.fetch_add(1, Relaxed) + 1;
                            max.fetch_max(concurrent, Relaxed);
                            thread::sleep(Duration::from_millis(if concurrent >= 3 { 1050 } else { 100 }));
                            active.fetch_sub(1, Relaxed);
                            let body = r#"{"answers":{"typo":{"choice":"yes"}}}"#;
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        });
                    }
                });
            });
            let result = measure(
                &url,
                "synthetic-key",
                None,
                true,
                Instant::now() + Duration::from_secs(9),
                || false,
            );
            stop.store(true, Relaxed);
            server.join().unwrap();
            let result = result.unwrap();
            assert_eq!(result.limits.concurrent, 2);
            assert_eq!(max.load(Relaxed), 4);
            assert_eq!(result.limits.requests_per_second, result.throughput * 0.8);
            assert!(result.p95_ms < 1000);
        });
    }
}
