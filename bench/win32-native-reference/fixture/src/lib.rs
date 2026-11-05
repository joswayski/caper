//! Deterministic Caper-like conversation data and the benchmark script both
//! UI implementations share, so each one draws the same screen and is driven
//! through exactly the same timeline.

use std::fmt::Write as _;

#[cfg(windows)]
pub mod sampler;
use std::time::{Duration, Instant};

pub const MESSAGES: usize = 5_000;
pub const AUTHORS: usize = 48;
pub const CHANNELS: usize = 36;

pub const BLACKOUT: u32 = 0x0C0D0F;
pub const SURFACE: u32 = 0x151719;
pub const BORDER: u32 = 0x34383B;
pub const TEXT: u32 = 0xF3F4F5;
pub const MUTED: u32 = 0x9A9EA3;
pub const EGGSHELL: u32 = 0xF1EDE4;
pub const TERRACOTTA: u32 = 0xB64D32;
pub const GREEN: u32 = 0x637A43;
pub const LINK: u32 = 0x9CC9F5;

pub struct Author {
    pub name: String,
    pub color: u32,
    pub initials: String,
}

pub enum Seg {
    Text(String),
    Bold(String),
    Italic(String),
    Link(String),
    Mention(usize),
}

pub struct Message {
    pub id: String,
    pub author: usize,
    pub time: String,
    /// Same author within five minutes of the previous message.
    pub grouped: bool,
    pub segs: Vec<Seg>,
    /// Attached picture size (procedurally generated pixels).
    pub image: Option<(u32, u32, u32)>,
    pub reactions: Vec<(&'static str, u32)>,
}

pub struct Fixture {
    pub authors: Vec<Author>,
    pub messages: Vec<Message>,
    pub channels: Vec<String>,
    next: u64,
    rng: Rng,
    minute: u32,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const FIRST: &[&str] = &[
    "Ana", "Bruno", "Chen", "Dalia", "Emeka", "Fatima", "Gus", "Hana", "Ivan", "Jules", "Kai",
    "Lena", "Mateo", "Nia", "Omar", "Priya", "Quinn", "Rosa", "Sam", "Tomás", "Uma", "Viktor",
    "Wren", "Yuki",
];
const LAST: &[&str] = &["Okafor", "Silva", "Nakamura", "Kowalski", "Haddad", "Reyes"];
const WORDS: &[&str] = &[
    "the", "voice", "channel", "is", "pretty", "good", "today", "but", "we", "should", "probably",
    "ship", "it", "after", "lunch", "anyone", "seen", "my", "headphones", "latency", "feels",
    "lower", "than", "yesterday", "release", "notes", "look", "great", "can", "you", "check",
    "staging", "again", "I", "think", "the", "invite", "link", "expired", "already", "lol",
    "honestly", "that", "was", "fast", "nice", "work", "everyone", "meeting", "moved", "to",
    "tomorrow", "morning", "coffee", "first", "then", "bugs", "screenshots", "attached", "below",
    "server", "restarted", "fine", "now", "thanks", "for", "the", "quick", "fix", "who", "owns",
    "this", "space", "game", "night", "friday", "playlist", "updated", "with", "new", "songs",
];
const EMOJI: &[&str] = &[
    "😅", "🎉", "👍", "✨", "🤔", "🚀", "🙌🏽", "☕", "👨‍👩‍👧", "🔥", "💯", "😂",
];
const REACTIONS: &[&str] = &["👍", "🎉", "😂", "🔥", "❤️", "👀"];
/// Real chat history is multilingual; these exercise shaping and fallback.
const INTERNATIONAL: &[&str] = &[
    "今日のリリースは最高でした！",
    "我们明天早上开会吧",
    "회의는 내일 아침으로 옮겼어요",
    "مرحبا بالجميع، كيف حالكم؟",
    "शाम को मिलते हैं, ठीक है?",
    "Привет всем, сервер снова работает",
    "שלום לכולם, הכול עובד עכשיו",
];

impl Fixture {
    pub fn new(count: usize) -> Self {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let authors = (0..AUTHORS)
            .map(|i| {
                let first = FIRST[i % FIRST.len()];
                let last = LAST[(i / FIRST.len() + i) % LAST.len()];
                let color = [0x637A43, 0xB64D32, 0x4D6FB6, 0x8A5BB0, 0xC29A3A, 0x3A9C9A][i % 6];
                Author {
                    name: format!("{first} {last}"),
                    color,
                    initials: format!("{}{}", &first[..1], &last[..1]),
                }
            })
            .collect();
        let channels = (0..CHANNELS)
            .map(|i| {
                [
                    "general", "random", "release", "voice-lab", "design", "music", "games",
                    "support", "feedback", "photos", "off-topic", "planning",
                ][i % 12]
                    .to_string()
                    + &if i >= 12 { format!("-{}", i / 12) } else { String::new() }
            })
            .collect();
        rng.next();
        let mut fixture = Self {
            authors,
            messages: Vec::with_capacity(count + 512),
            channels,
            next: 0,
            rng,
            minute: 9 * 60,
        };
        for _ in 0..count {
            fixture.push();
        }
        if std::env::var_os("INTL").is_some() {
            // One message exercising shaping, fallback and bidi, at the live end.
            let last = fixture.messages.last_mut().unwrap();
            last.segs = vec![Seg::Text(format!(
                "{} — 🙌🏽 👨‍👩‍👧 🇯🇵 — Hello عالم 123 שלום!",
                INTERNATIONAL.join(" · ")
            ))];
            last.image = None;
        }
        fixture
    }

    /// Appends one generated message (also how live arrivals are simulated).
    pub fn push(&mut self) {
        let rng = &mut self.rng;
        let previous = self.messages.last().map(|m| m.author);
        let author = if previous.is_some() && rng.chance(45) {
            previous.unwrap()
        } else {
            rng.below(AUTHORS as u64) as usize
        };
        let gap = if rng.chance(70) { 1 } else { 1 + rng.below(12) as u32 };
        self.minute += gap;
        let grouped = previous == Some(author) && gap < 5;
        let mut segs = Vec::new();
        let words = match rng.below(10) {
            0..=4 => 3 + rng.below(10),
            5..=7 => 12 + rng.below(25),
            _ => 40 + rng.below(70),
        };
        let mut buffer = String::new();
        let flush = |buffer: &mut String, segs: &mut Vec<Seg>| {
            if !buffer.is_empty() {
                segs.push(Seg::Text(std::mem::take(buffer)));
            }
        };
        for w in 0..words {
            if w > 0 {
                buffer.push(' ');
            }
            match rng.below(100) {
                0..=2 => {
                    flush(&mut buffer, &mut segs);
                    segs.push(Seg::Bold(format!("{} {}", pick(rng, WORDS), pick(rng, WORDS))));
                }
                3..=4 => {
                    flush(&mut buffer, &mut segs);
                    segs.push(Seg::Italic(pick(rng, WORDS).to_string()));
                }
                5 => {
                    flush(&mut buffer, &mut segs);
                    segs.push(Seg::Mention(rng.below(AUTHORS as u64) as usize));
                }
                6 => {
                    flush(&mut buffer, &mut segs);
                    let n = rng.below(9000);
                    segs.push(Seg::Link(format!("https://caper.chat/s/{n}")));
                }
                7..=9 => buffer.push_str(pick(rng, EMOJI)),
                10 if rng.chance(30) => buffer.push_str(pick(rng, INTERNATIONAL)),
                _ => buffer.push_str(pick(rng, WORDS)),
            }
        }
        flush(&mut buffer, &mut segs);
        let image = rng.chance(4).then(|| {
            let w = 240 + rng.below(160) as u32;
            (w, w * (5 + rng.below(4) as u32) / 10, rng.next() as u32)
        });
        let reactions = if rng.chance(12) {
            (0..1 + rng.below(3))
                .map(|i| (REACTIONS[(i as usize * 2 + author) % REACTIONS.len()], 1 + rng.below(9) as u32))
                .collect()
        } else {
            Vec::new()
        };
        let id = format!("m{:06}", self.next);
        self.next += 1;
        let (h, m) = ((self.minute / 60) % 12, self.minute % 60);
        self.messages.push(Message {
            id,
            author,
            time: format!("{}:{m:02} {}", if h == 0 { 12 } else { h }, if (self.minute / 60) % 24 < 12 { "AM" } else { "PM" }),
            grouped,
            segs,
            image,
            reactions,
        });
    }

    /// The message as Markdown (Caper's wire format, with pills as code spans).
    pub fn markdown(&self, message: &Message) -> String {
        let mut out = String::new();
        for seg in &message.segs {
            match seg {
                Seg::Text(t) => out.push_str(t),
                Seg::Bold(t) => {
                    let _ = write!(out, "**{t}**");
                }
                Seg::Italic(t) => {
                    let _ = write!(out, "*{t}*");
                }
                Seg::Link(u) => {
                    let _ = write!(out, "[{u}]({u})");
                }
                Seg::Mention(a) => {
                    let _ = write!(out, "`@{}`", self.authors[*a].name);
                }
            }
        }
        out
    }
}

fn pick<'a>(rng: &mut Rng, items: &[&'a str]) -> &'a str {
    items[rng.below(items.len() as u64) as usize]
}

/// Procedural RGBA pixels for an attachment (decoded once, like a real image).
pub fn image_pixels(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    let (r0, g0, b0) = (seed & 0xff, (seed >> 8) & 0xff, (seed >> 16) & 0xff);
    for y in 0..h {
        for x in 0..w {
            let v = ((x * 255 / w) ^ (y * 255 / h)) & 0xff;
            out.extend_from_slice(&[((r0 + v) / 2) as u8, ((g0 + x % 97) / 2) as u8, ((b0 + y % 89) / 2) as u8, 255]);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Benchmark script

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    /// Interactive: no script.
    None,
    /// Launch, settle, measure memory.
    Startup,
    /// Nothing happens for 10 s.
    Idle,
    /// Fling up through history at 2000 px/s for 8 s.
    Scroll,
    /// A new message every 100 ms for 10 s, following the live end.
    Burst,
    /// Window width sweeps 1000–1500 px for 6 s.
    Resize,
    /// 20 characters per second into the composer for 8 s.
    Typing,
    /// External pointer sweeps (xdotool) for 8 s; the app only measures.
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
    ticks: (f64, f64),
    pub framework: &'static str,
    pub extra: Vec<(String, f64)>,
}

const SETTLE: f64 = 2.0;

impl Bench {
    pub fn from_env(framework: &'static str, process_start: Instant) -> Self {
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
        #[cfg(windows)]
        sampler::start();
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
            ticks: (0.0, 0.0),
            framework,
            extra: Vec::new(),
        }
    }

    pub fn measuring(&self) -> bool {
        self.measuring
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

    /// Whether the app must keep producing frames without input (scripted
    /// motion). Idle and hover must not force frames.
    pub fn wants_frames(&self) -> bool {
        matches!(self.scenario, Scenario::Scroll | Scenario::Resize | Scenario::Burst | Scenario::Typing | Scenario::Startup)
            || (self.active() && !self.measuring && self.measure_end.is_none())
    }

    /// Seconds until the script next needs a frame (for idle/hover: the end).
    pub fn wake_in(&self) -> f64 {
        let t = self.start.elapsed().as_secs_f64();
        (SETTLE + self.length() - t).max(0.0) + 0.01
    }

    /// Call once per rendered frame; returns what the app must do this frame.
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
            self.measure_start = Some((now, cpu_seconds())); self.ticks.0 = cpu_seconds_ticks(); #[cfg(windows)] sampler::set_active(true);
        }
        if self.measuring {
            self.frames.push(now);
            let elapsed = t - SETTLE;
            if elapsed >= self.length() {
                self.measuring = false;
                self.measure_end = Some((now, cpu_seconds())); self.ticks.1 = cpu_seconds_ticks();
                self.write();
                return Action::Quit;
            }
            return match self.scenario {
                Scenario::Scroll => Action::ScrollUp((2000.0 * dt) as f32),
                Scenario::Burst => {
                    let due = (elapsed / 0.1) as usize;
                    if self.appended < due {
                        self.appended += 1;
                        Action::Append
                    } else {
                        Action::None
                    }
                }
                Scenario::Resize => {
                    let phase = (elapsed * std::f64::consts::PI).sin() as f32;
                    Action::Resize(1250.0 + 250.0 * phase, 820.0)
                }
                Scenario::Typing => {
                    let due = (elapsed * 20.0) as usize;
                    if self.typed < due {
                        self.typed += 1;
                        let text = "this is a pretty long draft message with some words to wrap ";
                        Action::Type(text.as_bytes()[self.typed % text.len()] as char)
                    } else {
                        Action::None
                    }
                }
                _ => Action::None,
            };
        }
        Action::None
    }

    /// Idle/hover: called from a timer instead of a frame, so the scenario
    /// can end without the app rendering.
    pub fn tick(&mut self) -> bool {
        let t = self.start.elapsed().as_secs_f64();
        if self.active() && !self.measuring && self.measure_end.is_none() && t >= SETTLE {
            self.measuring = true;
            self.measure_start = Some((Instant::now(), cpu_seconds())); self.ticks.0 = cpu_seconds_ticks(); #[cfg(windows)] sampler::set_active(true);
        }
        if self.measuring && t - SETTLE >= self.length() {
            self.measuring = false;
            self.measure_end = Some((Instant::now(), cpu_seconds())); self.ticks.1 = cpu_seconds_ticks();
            self.write();
            return true;
        }
        false
    }

    fn write(&self) {
        #[cfg(windows)]
        sampler::dump();
        let (Some((t0, c0)), Some((t1, c1))) = (self.measure_start, self.measure_end) else {
            return;
        };
        let wall = (t1 - t0).as_secs_f64();
        let cpu = c1 - c0;
        let mut intervals: Vec<f64> = self.frames.windows(2).map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0).collect();
        intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |p: f64| -> f64 {
            if intervals.is_empty() {
                0.0
            } else {
                intervals[((intervals.len() - 1) as f64 * p).round() as usize]
            }
        };
        let frames = self.frames.len();
        let (rss, pss) = memory_mib();
        let mut json = format!(
            "{{\"framework\":\"{}\",\"scenario\":\"{:?}\",\"wall_s\":{wall:.3},\"cpu_s\":{cpu:.3},\"cpu_pct\":{:.1},\"frames\":{frames},\"fps\":{:.1},\"cpu_ms_per_frame\":{:.3},\"frame_p50_ms\":{:.2},\"frame_p95_ms\":{:.2},\"frame_p99_ms\":{:.2},\"frame_max_ms\":{:.2},\"over_33ms\":{},\"rss_mib\":{rss:.1},\"pss_mib\":{pss:.1},\"first_frame_ms\":{:.1}",
            self.framework,
            self.scenario,
            cpu / wall.max(1e-9) * 100.0,
            frames as f64 / wall.max(1e-9),
            if frames > 0 { cpu * 1000.0 / frames as f64 } else { 0.0 },
            pct(0.5),
            pct(0.95),
            pct(0.99),
            intervals.last().copied().unwrap_or(0.0),
            intervals.iter().filter(|i| **i > 33.4).count(),
            self.first_frame.map_or(0.0, |d| d.as_secs_f64() * 1000.0),
        );
        for (k, v) in &self.extra {
            let _ = write!(json, ",\"{k}\":{v:.3}");
        }
        let _ = write!(json, ",\"cpu_pct_ticks\":{:.1}", (self.ticks.1 - self.ticks.0) / wall.max(1e-9) * 100.0);
        json.push('}');
        match std::env::var("BENCH_OUT") {
            Ok(path) => {
                use std::io::Write as _;
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
                writeln!(f, "{json}").unwrap();
            }
            Err(_) => println!("{json}"),
        }
    }
}

/// Windows: CPU of the whole process from its cycle count (exact; the
/// driver passes the calibrated TSC rate in TSC_HZ).
#[cfg(windows)]
pub fn cpu_seconds() -> f64 {
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryProcessCycleTime(process: *mut core::ffi::c_void, cycles: *mut u64) -> i32;
    }
    let hz: f64 = std::env::var("TSC_HZ").ok().and_then(|v| v.parse().ok()).expect("TSC_HZ");
    let mut cycles = 0u64;
    unsafe { QueryProcessCycleTime(GetCurrentProcess(), &mut cycles) };
    cycles as f64 / hz
}

#[cfg(unix)]
pub fn cpu_seconds_ticks() -> f64 {
    cpu_seconds()
}

/// User + system CPU of the whole process (all threads), in seconds, as
/// Windows charges it at clock ticks (misses short bursts).
#[cfg(windows)]
pub fn cpu_seconds_ticks() -> f64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    let ft = |t: FILETIME| ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 1e7;
    ft(k) + ft(u)
}

/// User + system CPU of the whole process (all threads), in seconds.
#[cfg(unix)]
pub fn cpu_seconds() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(usage.ru_utime) + tv(usage.ru_stime)
}

/// (RSS, PSS on Linux / physical footprint on macOS) in MiB.
pub fn memory_mib() -> (f64, f64) {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
        let field = |name: &str| {
            text.lines()
                .find(|l| l.starts_with(name))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 1024.0
        };
        (field("Rss:"), field("Pss:"))
    }
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            libc::proc_pid_rusage(
                libc::getpid(),
                libc::RUSAGE_INFO_V4,
                &mut info as *mut _ as *mut libc::rusage_info_t,
            )
        };
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        let footprint = if ok == 0 { info.ri_phys_footprint as f64 / 1048576.0 } else { 0.0 };
        (info.ri_resident_size as f64 / 1048576.0, footprint)
    }
    // Windows: (working set, private bytes). GPU-dedicated memory is not
    // included; the Windows driver samples it from GPU Process Memory counters.
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut info: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        info.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut info as *mut _ as *mut _, info.cb) };
        if ok == 0 {
            return (0.0, 0.0);
        }
        (info.WorkingSetSize as f64 / 1048576.0, info.PrivateUsage as f64 / 1048576.0)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        (0.0, 0.0)
    }
}
