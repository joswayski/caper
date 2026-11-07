//! Scripted performance scenarios for development, inert unless `BENCH` is
//! set to `startup`, `idle`, `scroll` or `typing`. The same scenarios and
//! JSON line as the Windows client's (`apps/native/windows/src/bench.rs`),
//! so their results compare. A real client signs in and loads history
//! first, so the 2 s settle starts once a drawn frame has shown the
//! conversation's messages (`ready`, reported as `content_ms`). Then the scenario runs for its length, one JSON line goes to
//! `BENCH_OUT` (or stdout) and the app quits. CPU (all threads, user and
//! system) and memory come from `/proc/self`; elsewhere they read as zero.
use std::fmt::Write as _;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    None,
    Startup,
    Idle,
    Scroll,
    Typing,
}

/// What the client does now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    None,
    /// Scroll the messages up by this many logical pixels.
    ScrollUp(f32),
    /// Type this character into the composer.
    Type(char),
    Quit,
}

const SETTLE: f64 = 2.0;
const TYPED: &[u8] = b"this is a pretty long draft message with some words to wrap ";

pub struct Bench {
    pub scenario: Scenario,
    framework: &'static str,
    process_start: Instant,
    first_frame: Option<Duration>,
    ready: Option<Instant>,
    showing: bool,
    frames: Vec<Instant>,
    measure_start: Option<(Instant, f64)>,
    done: bool,
    last: Option<Instant>,
    typed: usize,
}

impl Bench {
    /// `framework` names the client in the JSON line; `process_start` is
    /// taken first thing in `main`.
    pub fn from_env(framework: &'static str, process_start: Instant) -> Self {
        let scenario = match std::env::var("BENCH").as_deref() {
            Ok("startup") => Scenario::Startup,
            Ok("idle") => Scenario::Idle,
            Ok("scroll") => Scenario::Scroll,
            Ok("typing") => Scenario::Typing,
            _ => Scenario::None,
        };
        Self {
            scenario,
            framework,
            process_start,
            first_frame: None,
            ready: None,
            showing: false,
            frames: Vec::new(),
            measure_start: None,
            done: false,
            last: None,
            typed: 0,
        }
    }

    pub fn active(&self) -> bool {
        self.scenario != Scenario::None
    }

    fn length(&self) -> f64 {
        match self.scenario {
            Scenario::None => f64::INFINITY,
            Scenario::Startup => 0.0,
            Scenario::Idle => 10.0,
            Scenario::Scroll | Scenario::Typing => 8.0,
        }
    }

    /// For a client that learns a frame was drawn only when the next one
    /// starts: whether this frame shows the messages. True (and ready) once
    /// the frame before it did.
    pub fn showing(&mut self, showing: bool) -> bool {
        let shown = std::mem::replace(&mut self.showing, showing);
        shown && self.ready()
    }

    /// A drawn frame has shown the conversation's messages. True the first
    /// time.
    pub fn ready(&mut self) -> bool {
        if !self.active() || self.ready.is_some() {
            return false;
        }
        self.ready = Some(Instant::now());
        true
    }

    /// Whether the script needs a frame every refresh: once ready and until
    /// done. Idle never does.
    pub fn wants_frames(&self) -> bool {
        self.ready.is_some()
            && !self.done
            && matches!(
                self.scenario,
                Scenario::Startup | Scenario::Scroll | Scenario::Typing
            )
    }

    /// When `step` must next run even if nothing draws.
    pub fn next_check(&self) -> Option<Duration> {
        let ready = self.ready.filter(|_| !self.done)?;
        let at = match self.measure_start {
            None => ready + Duration::from_secs_f64(SETTLE),
            Some((start, _)) => start + Duration::from_secs_f64(self.length()),
        };
        Some(at.saturating_duration_since(Instant::now()))
    }

    /// Once per drawn frame.
    pub fn painted(&mut self) {
        let now = Instant::now();
        if self.first_frame.is_none() {
            self.first_frame = Some(now - self.process_start);
        }
        if self.measure_start.is_some() && !self.done {
            self.frames.push(now);
        }
    }

    /// Once per frame (and at `next_check`): what the script does now.
    pub fn step(&mut self) -> Action {
        let now = Instant::now();
        let Some(ready) = self.ready.filter(|_| self.active() && !self.done) else {
            return Action::None;
        };
        let t = (now - ready).as_secs_f64();
        let dt = self.last.map_or(0.0, |l| (now - l).as_secs_f64()).min(0.1);
        self.last = Some(now);
        if self.measure_start.is_none() {
            if t < SETTLE {
                return Action::None;
            }
            self.measure_start = Some((now, cpu_seconds()));
        }
        let Some((start, _)) = self.measure_start else {
            return Action::None;
        };
        let elapsed = (now - start).as_secs_f64();
        if elapsed >= self.length() {
            self.finish(now);
            return Action::Quit;
        }
        match self.scenario {
            Scenario::Scroll => Action::ScrollUp((2000.0 * dt) as f32),
            Scenario::Typing if self.typed < (elapsed * 20.0) as usize => {
                self.typed += 1;
                Action::Type(TYPED[self.typed % TYPED.len()] as char)
            }
            _ => Action::None,
        }
    }

    fn finish(&mut self, now: Instant) {
        self.done = true;
        let Some((t0, c0)) = self.measure_start else {
            return;
        };
        let json = self.report((now - t0).as_secs_f64().max(1e-9), cpu_seconds() - c0);
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

    fn report(&self, wall: f64, cpu: f64) -> String {
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
        let (rss, pss) = memory_mib();
        let ms = |d: Option<Duration>| d.map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let mut json = String::new();
        let _ = write!(
            json,
            "{{\"framework\":\"{}\",\"scenario\":\"{:?}\",\"wall_s\":{wall:.3},\"cpu_s\":{cpu:.3},\"cpu_pct\":{:.1},\"frames\":{frames},\"fps\":{:.1},\"cpu_ms_per_frame\":{:.3},\"frame_p50_ms\":{:.2},\"frame_p95_ms\":{:.2},\"frame_p99_ms\":{:.2},\"frame_max_ms\":{:.2},\"over_33ms\":{},\"rss_mib\":{rss:.1},\"pss_mib\":{pss:.1},\"first_frame_ms\":{:.1},\"content_ms\":{:.1}}}",
            self.framework,
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
            ms(self.first_frame),
            ms(self.ready.map(|r| r - self.process_start)),
        );
        json
    }
}

/// The process's user and system CPU, every thread's, from `/proc/self/stat`.
fn cpu_seconds() -> f64 {
    std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| parse_cpu(&stat))
        .unwrap_or(0.0)
}

fn parse_cpu(stat: &str) -> Option<f64> {
    // Fields after the parenthesised name start at the state (field 3);
    // utime and stime are fields 14 and 15, in USER_HZ (100) ticks.
    let mut fields = stat.rsplit_once(')')?.1.split_whitespace().skip(11);
    let utime: u64 = fields.next()?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some((utime + stime) as f64 / 100.0)
}

/// (resident, proportional) set size in MiB, from `/proc/self/smaps_rollup`.
pub fn memory_mib() -> (f64, f64) {
    std::fs::read_to_string("/proc/self/smaps_rollup")
        .map(|rollup| parse_memory(&rollup))
        .unwrap_or((0.0, 0.0))
}

fn parse_memory(rollup: &str) -> (f64, f64) {
    let field = |name: &str| {
        rollup
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next()?.parse::<f64>().ok())
            .map_or(0.0, |kib| kib / 1024.0)
    };
    (field("Rss:"), field("Pss:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cpu_past_a_name_with_spaces_and_parentheses() {
        // pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt
        // majflt cmajflt utime stime ...
        let stat = "42 (caper (gtk) x) S 1 42 42 0 -1 4194560 10 0 0 0 250 75 0 0 20 0";
        assert_eq!(parse_cpu(stat), Some(3.25));
    }

    #[test]
    fn reads_resident_and_proportional_sizes() {
        let rollup = "55d0-7ffd ---p 00000000 00:00 0 [rollup]\nRss:              204800 kB\nPss:              102400 kB\nPss_Anon:          51200 kB\n";
        assert_eq!(parse_memory(rollup), (200.0, 100.0));
    }
}
