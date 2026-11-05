//! Scripted performance scenarios for development, inert unless `BENCH` is
//! set: startup, idle, scroll, burst, resize, typing, hover. After a 2 s
//! settle the scenario runs for its length, then one JSON line goes to
//! `BENCH_OUT` (or stdout) and the app quits. Process CPU is counted in
//! cycles (`TSC_HZ` gives the calibrated rate) so short bursts are not lost.
//! `CAPER_BENCH_FRAMES=<file>` also logs every frame's duration.
use std::fmt::Write as _;
use std::time::{Duration, Instant};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::GetCurrentProcess;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryProcessCycleTime(process: windows::Win32::Foundation::HANDLE, cycles: *mut u64) -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    None,
    Startup,
    Idle,
    Scroll,
    Burst,
    Resize,
    Typing,
    Hover,
}

pub enum Action {
    None,
    ScrollUp(f32),
    Append,
    Resize(f32, f32),
    Type(char),
    Quit,
}

const SETTLE: f64 = 2.0;

pub struct Bench {
    pub scenario: Scenario,
    start: Instant,
    process_start: Instant,
    first_frame: Option<Duration>,
    frames: Vec<Instant>,
    measuring: bool,
    measure_start: Option<(Instant, f64)>,
    measure_end: Option<(Instant, f64)>,
    last: Option<Instant>,
    typed: usize,
    appended: usize,
    frame_log: Option<std::io::BufWriter<std::fs::File>>,
}

impl Bench {
    pub fn from_env(process_start: Instant) -> Self {
        let scenario = match std::env::var("BENCH").as_deref() {
            Ok("startup") => Scenario::Startup,
            Ok("idle") => Scenario::Idle,
            Ok("scroll") => Scenario::Scroll,
            Ok("burst") => Scenario::Burst,
            Ok("resize") => Scenario::Resize,
            Ok("typing") => Scenario::Typing,
            Ok("hover") => Scenario::Hover,
            _ => Scenario::None,
        };
        let frame_log = std::env::var("CAPER_BENCH_FRAMES")
            .ok()
            .and_then(|p| std::fs::File::create(p).ok())
            .map(std::io::BufWriter::new);
        Self {
            scenario,
            start: Instant::now(),
            process_start,
            first_frame: None,
            frames: Vec::new(),
            measuring: false,
            measure_start: None,
            measure_end: None,
            last: None,
            typed: 0,
            appended: 0,
            frame_log,
        }
    }

    pub fn active(&self) -> bool {
        self.scenario != Scenario::None
    }

    fn length(&self) -> f64 {
        match self.scenario {
            Scenario::None => f64::INFINITY,
            Scenario::Startup => 0.0,
            Scenario::Idle | Scenario::Burst => 10.0,
            Scenario::Resize => 6.0,
            Scenario::Scroll | Scenario::Typing | Scenario::Hover => 8.0,
        }
    }

    /// Whether the script needs frames without input. Idle and hover never do.
    pub fn wants_frames(&self) -> bool {
        matches!(
            self.scenario,
            Scenario::Scroll
                | Scenario::Resize
                | Scenario::Burst
                | Scenario::Typing
                | Scenario::Startup
        ) || (self.active()
            && !self.measuring
            && self.measure_end.is_none()
            && self.scenario != Scenario::Idle
            && self.scenario != Scenario::Hover)
    }

    /// Duration of one rendered frame, for `CAPER_BENCH_FRAMES`.
    pub fn log_frame(&mut self, took: Duration) {
        use std::io::Write;
        if let Some(f) = &mut self.frame_log {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros();
            let _ = writeln!(f, "{now} {}", took.as_micros());
            let _ = f.flush();
        }
    }

    /// Once per rendered frame: what the script does this frame.
    pub fn frame(&mut self) -> Action {
        let now = Instant::now();
        if self.first_frame.is_none() {
            self.first_frame = Some(now - self.process_start);
        }
        if !self.active() {
            return Action::None;
        }
        let t = (now - self.start).as_secs_f64();
        let dt = self.last.map_or(0.0, |l| (now - l).as_secs_f64()).min(0.1);
        self.last = Some(now);
        if !self.measuring && self.measure_end.is_none() && t >= SETTLE {
            self.measuring = true;
            self.measure_start = Some((now, cpu_seconds()));
        }
        if !self.measuring {
            return Action::None;
        }
        self.frames.push(now);
        let elapsed = t - SETTLE;
        if elapsed >= self.length() {
            self.finish(now);
            return Action::Quit;
        }
        match self.scenario {
            Scenario::Scroll => Action::ScrollUp((2000.0 * dt) as f32),
            Scenario::Burst if self.appended < (elapsed / 0.1) as usize => {
                self.appended += 1;
                Action::Append
            }
            Scenario::Resize => Action::Resize(
                1250.0 + 250.0 * (elapsed * std::f64::consts::PI).sin() as f32,
                820.0,
            ),
            Scenario::Typing if self.typed < (elapsed * 20.0) as usize => {
                self.typed += 1;
                let text = b"this is a pretty long draft message with some words to wrap ";
                Action::Type(text[self.typed % text.len()] as char)
            }
            _ => Action::None,
        }
    }

    /// Idle and hover end from a timer, so they finish without rendering.
    pub fn tick(&mut self) -> bool {
        let t = self.start.elapsed().as_secs_f64();
        if !matches!(self.scenario, Scenario::Idle | Scenario::Hover) {
            return false;
        }
        if !self.measuring && self.measure_end.is_none() && t >= SETTLE {
            self.measuring = true;
            self.measure_start = Some((Instant::now(), cpu_seconds()));
        }
        if self.measuring && t - SETTLE >= self.length() {
            self.finish(Instant::now());
            return true;
        }
        false
    }

    fn finish(&mut self, now: Instant) {
        self.measuring = false;
        self.measure_end = Some((now, cpu_seconds()));
        let (Some((t0, c0)), Some((t1, c1))) = (self.measure_start, self.measure_end) else {
            return;
        };
        let wall = (t1 - t0).as_secs_f64().max(1e-9);
        let cpu = c1 - c0;
        let mut intervals: Vec<f64> = self
            .frames
            .windows(2)
            .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
            .collect();
        intervals.sort_by(f64::total_cmp);
        let pct = |p: f64| {
            if intervals.is_empty() {
                0.0
            } else {
                intervals[((intervals.len() - 1) as f64 * p).round() as usize]
            }
        };
        let frames = self.frames.len();
        let (ws, private) = memory_mib();
        let mut json = String::new();
        let _ = write!(
            json,
            "{{\"framework\":\"caper-windows\",\"scenario\":\"{:?}\",\"wall_s\":{wall:.3},\"cpu_s\":{cpu:.3},\"cpu_pct\":{:.1},\"frames\":{frames},\"fps\":{:.1},\"cpu_ms_per_frame\":{:.3},\"frame_p50_ms\":{:.2},\"frame_p95_ms\":{:.2},\"frame_p99_ms\":{:.2},\"frame_max_ms\":{:.2},\"over_33ms\":{},\"rss_mib\":{ws:.1},\"pss_mib\":{private:.1},\"first_frame_ms\":{:.1}}}",
            self.scenario,
            cpu / wall * 100.0,
            frames as f64 / wall,
            if frames > 0 {
                cpu * 1000.0 / frames as f64
            } else {
                0.0
            },
            pct(0.5),
            pct(0.95),
            pct(0.99),
            intervals.last().copied().unwrap_or(0.0),
            intervals.iter().filter(|i| **i > 33.4).count(),
            self.first_frame.map_or(0.0, |d| d.as_secs_f64() * 1000.0),
        );
        match std::env::var("BENCH_OUT") {
            Ok(path) => {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "{json}");
                }
            }
            Err(_) => println!("{json}"),
        }
    }
}

/// CPU of the whole process from its cycle count.
fn cpu_seconds() -> f64 {
    let hz: f64 = std::env::var("TSC_HZ")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3.8e9);
    let mut cycles = 0u64;
    unsafe {
        let _ = QueryProcessCycleTime(GetCurrentProcess(), &mut cycles);
    }
    cycles as f64 / hz
}

/// (working set, private bytes) in MiB.
pub fn memory_mib() -> (f64, f64) {
    let mut info = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut info).cast::<PROCESS_MEMORY_COUNTERS>(),
            info.cb,
        )
    };
    if ok.is_err() {
        return (0.0, 0.0);
    }
    (
        info.WorkingSetSize as f64 / 1048576.0,
        info.PrivateUsage as f64 / 1048576.0,
    )
}
