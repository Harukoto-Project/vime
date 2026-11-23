use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    mem::size_of,
    path::Path,
    sync::mpsc::{SyncSender, sync_channel},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::FILETIME,
    System::{
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

pub struct Diagnostics {
    send: Option<SyncSender<Sample>>,
    since: Instant,
    last_logic: Option<Instant>,
    last_ui: Option<Instant>,
    state: OverlayState,
    dropped: u64,
    pub counters: Counters,
}

impl Diagnostics {
    pub fn new(directory: &Path) -> Self {
        let (send, receive) = sync_channel::<Sample>(8);
        let directory = directory.to_path_buf();
        let worker = thread::Builder::new()
            .name("vime-diagnostics".into())
            .spawn(move || {
                for sample in receive {
                    if let Err(error) = write_sample(&directory, sample) {
                        eprintln!("ViME diagnostics: {error}");
                    }
                }
            });
        Self {
            send: worker.ok().map(|_| send),
            since: Instant::now(),
            last_logic: None,
            last_ui: None,
            state: OverlayState::default(),
            dropped: 0,
            counters: Counters::default(),
        }
    }

    pub fn start_logic(&mut self, now: Instant) {
        if let Some(previous) = self.last_logic.replace(now) {
            self.counters.logic_gap.observe(now - previous);
        }
    }

    pub fn start_ui(&mut self, now: Instant) {
        if let Some(previous) = self.last_ui.replace(now) {
            self.counters.ui_gap.observe(now - previous);
        }
    }

    pub fn state(&mut self, state: OverlayState) {
        self.counters.dashboard_vis_changes +=
            u64::from(self.state.dashboard_visible != state.dashboard_visible);
        self.counters.dashboard_active_changes +=
            u64::from(self.state.dashboard_active != state.dashboard_active);
        self.state = state;
    }

    pub fn sample(&mut self, now: Instant, scene_pid: impl FnOnce() -> u32) {
        let interval = now - self.since;
        if interval < SAMPLE_INTERVAL {
            return;
        }
        self.since = now;
        let counters = std::mem::take(&mut self.counters);
        if let Some(send) = &self.send {
            let sample = Sample {
                schema: 1,
                unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                pid: std::process::id(),
                interval_ms: interval.as_millis() as u64,
                scene_pid: scene_pid(),
                dropped_before: self.dropped,
                state: self.state,
                counters,
            };
            if send.try_send(sample).is_ok() {
                self.dropped = 0;
            } else {
                self.dropped += 1;
            }
        }
    }
}

#[derive(Clone, Copy, Default, Serialize)]
pub struct OverlayState {
    pub connected: bool,
    pub world_visible: bool,
    pub dashboard_visible: bool,
    pub dashboard_active: bool,
    pub desktop_minimized: bool,
}

#[derive(Default, Serialize)]
pub struct Counters {
    pub logic_gap: Timing,
    pub logic: Timing,
    pub ui_gap: Timing,
    pub ui: Timing,
    pub vr_poll: Timing,
    pub world_render: Timing,
    pub world_cursor: Timing,
    pub world_submit: Timing,
    pub world_openvr: Timing,
    pub dashboard_render: Timing,
    pub dashboard_submit: Timing,
    pub dashboard_openvr: Timing,
    pub paint_requests: u64,
    pub pointer_observations: u64,
    pub presses: u64,
    pub dashboard_events: u64,
    pub dashboard_vis_changes: u64,
    pub dashboard_active_changes: u64,
    pub world_resumes: u64,
    pub dashboard_resumes: u64,
    pub input_errors: u64,
    pub last_input_error: Option<i32>,
    pub world_submit_errors: u64,
    pub last_world_submit_error: Option<String>,
    pub dashboard_submit_errors: u64,
    pub last_dashboard_submit_error: Option<String>,
}

#[derive(Default, Serialize)]
pub struct Timing {
    count: u64,
    total_us: u64,
    max_us: u64,
}

impl Timing {
    pub fn observe(&mut self, duration: Duration) {
        let micros = duration.as_micros() as u64;
        self.count += 1;
        self.total_us += micros;
        self.max_us = self.max_us.max(micros);
    }
}

#[derive(Serialize)]
struct Sample {
    schema: u8,
    unix_ms: u64,
    pid: u32,
    interval_ms: u64,
    scene_pid: u32,
    dropped_before: u64,
    state: OverlayState,
    counters: Counters,
}

#[derive(Serialize)]
struct ProcessStats {
    working_set_bytes: Option<usize>,
    commit_bytes: Option<usize>,
    page_faults: Option<u32>,
    cpu_ms_total: Option<u64>,
}

fn process_stats() -> ProcessStats {
    fn ticks(time: FILETIME) -> u64 {
        (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
    }

    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    let [mut created, mut exited, mut kernel, mut user] = [FILETIME::default(); 4];
    // The pseudo handle is valid for the current process and these buffers have the API's layout.
    let (has_memory, has_times) = unsafe {
        let process = GetCurrentProcess();
        (
            K32GetProcessMemoryInfo(
                process,
                &mut memory,
                size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ) != 0,
            GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) != 0,
        )
    };
    ProcessStats {
        working_set_bytes: has_memory.then_some(memory.WorkingSetSize),
        commit_bytes: has_memory.then_some(memory.PagefileUsage),
        page_faults: has_memory.then_some(memory.PageFaultCount),
        cpu_ms_total: has_times.then(|| (ticks(kernel) + ticks(user)) / 10_000),
    }
}

fn write_sample(directory: &Path, sample: Sample) -> io::Result<()> {
    #[derive(Serialize)]
    struct Record {
        #[serde(flatten)]
        sample: Sample,
        process: ProcessStats,
    }

    fs::create_dir_all(directory)?;
    let log = directory.join("diagnostics.jsonl");
    if fs::metadata(&log).is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES) {
        let previous = directory.join("diagnostics.previous.jsonl");
        match fs::remove_file(&previous) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(&log, previous)?;
    }
    let mut line = serde_json::to_vec(&Record {
        sample,
        process: process_stats(),
    })?;
    line.push(b'\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?
        .write_all(&line)
}
