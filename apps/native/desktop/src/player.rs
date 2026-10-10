//! In-app video playback. The bundled FFmpeg decodes in two processes: video
//! as raw RGBA frames at a constant rate, audio as 48 kHz stereo float PCM.
//! Audio plays through rodio and is the clock; frames show when the clock
//! reaches them. Without audio, a wall clock stands in. Seeking restarts both
//! decoders at the new time.

use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use rodio::{OutputStreamBuilder, Sink, Source};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// Samples per audio chunk sent from the decoder (about 43 ms of stereo).
const CHUNK_SAMPLES: usize = 4096;
/// About 1.4 s of audio and three frames are buffered ahead.
const AUDIO_CHUNKS: usize = 32;
const VIDEO_FRAMES: usize = 3;
const DEFAULT_FPS: f32 = 30.0;
const MAX_FPS: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Loading,
    Playing,
    Paused,
    Ended,
}

/// What the decoders need: an FFmpeg input (a file path or a loopback URL)
/// and the display size to scale to.
pub struct Player {
    binary: std::path::PathBuf,
    input: String,
    /// Output frame size (even, at most the video's own size).
    size: [usize; 2],
    duration_ms: Option<u64>,
    fps: f32,
    state: State,
    /// Where playback starts or stays while paused.
    position_ms: u64,
    volume: f32,
    muted: bool,
    session: Option<Session>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
}

struct Frame {
    pts_ms: u64,
    pixels: Vec<u8>,
}

struct Session {
    start_ms: u64,
    frames: Receiver<Frame>,
    next: Option<Frame>,
    video_done: bool,
    video: Arc<Mutex<Option<Child>>>,
    audio: Option<Audio>,
    /// Wall-clock fallback: when it (re)started and from which time.
    wall: Option<(Instant, u64)>,
    shown_any: bool,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut child) = self
            .video
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Player {
    /// `width`/`height` are the video's display size (rotation applied);
    /// `fit` bounds the decoded frames, which are never enlarged.
    pub fn new(
        binary: &std::path::Path,
        input: String,
        width: u32,
        height: u32,
        duration_ms: Option<u64>,
        fps: Option<f32>,
        fit: egui::Vec2,
    ) -> Self {
        Self {
            binary: binary.to_owned(),
            input,
            size: output_size(width, height, fit),
            duration_ms,
            fps: fps
                .filter(|fps| fps.is_finite() && *fps > 0.0)
                .map_or(DEFAULT_FPS, |fps| fps.min(MAX_FPS)),
            state: State::Loading,
            position_ms: 0,
            volume: 1.0,
            muted: false,
            session: None,
            texture: None,
            error: None,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// The decoded frame size, for laying out before the first frame.
    pub fn frame_size(&self) -> egui::Vec2 {
        egui::vec2(self.size[0] as f32, self.size[1] as f32)
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        self.apply_volume();
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.apply_volume();
    }

    fn apply_volume(&self) {
        if let Some(audio) = self
            .session
            .as_ref()
            .and_then(|session| session.audio.as_ref())
        {
            audio.command(AudioCommand::Volume(if self.muted {
                0.0
            } else {
                self.volume
            }));
        }
    }

    /// Current playback time.
    pub fn position_ms(&self) -> u64 {
        match (&self.session, self.state) {
            (Some(session), State::Playing) => session.clock_ms(),
            _ => self.position_ms,
        }
        .min(self.duration_ms.unwrap_or(u64::MAX))
    }

    pub fn play(&mut self) {
        match self.state {
            State::Playing => {}
            State::Ended => self.seek(0, true),
            State::Loading | State::Paused => match &mut self.session {
                Some(session) => {
                    session.resume(self.position_ms);
                    self.state = State::Playing;
                }
                None => self.start(self.position_ms, true),
            },
        }
    }

    pub fn pause(&mut self) {
        if self.state == State::Playing {
            self.position_ms = self.position_ms();
            if let Some(session) = &mut self.session {
                session.pause();
            }
            self.state = State::Paused;
        }
    }

    pub fn toggle(&mut self) {
        if self.state == State::Playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Jump to `ms`, keeping play/pause (a paused seek shows the new frame).
    pub fn seek(&mut self, ms: u64, play: bool) {
        let ms = ms.min(self.duration_ms.unwrap_or(u64::MAX));
        self.start(ms, play || self.state == State::Playing);
    }

    fn start(&mut self, ms: u64, play: bool) {
        self.session = None;
        self.error = None;
        self.position_ms = ms;
        match Session::start(self, ms, play) {
            Ok(session) => {
                self.session = Some(session);
                self.state = if play { State::Playing } else { State::Paused };
                self.apply_volume();
            }
            Err(error) => {
                self.error = Some(error);
                self.state = State::Paused;
            }
        }
    }

    /// Advance to the frame for the current time; returns when to repaint.
    pub fn update(&mut self, context: &egui::Context) -> Option<Duration> {
        let session = self.session.as_mut()?;
        let paused = self.state != State::Playing;
        let clock = if paused {
            self.position_ms
        } else {
            session.clock_ms()
        };
        let mut shown = None;
        loop {
            if session.next.is_none() && !session.video_done {
                match session.frames.try_recv() {
                    Ok(frame) => session.next = Some(frame),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => session.video_done = true,
                }
            }
            match &session.next {
                // Paused: show only the first frame at or after the seek.
                Some(frame) if frame.pts_ms <= clock || (paused && !session.shown_any) => {
                    shown = session.next.take();
                    if paused {
                        break;
                    }
                }
                _ => break,
            }
        }
        if let Some(frame) = shown {
            session.shown_any = true;
            let image = egui::ColorImage::from_rgba_unmultiplied(self.size, &frame.pixels);
            match &mut self.texture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.texture = Some(context.load_texture(
                        "video-frame",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        }
        if paused {
            return session.shown_any.then_some(Duration::from_millis(250));
        }
        let audio_done = session.audio.as_ref().is_none_or(Audio::finished);
        if session.video_done && session.next.is_none() && audio_done {
            self.position_ms = self.duration_ms.unwrap_or(clock);
            self.session = None;
            self.state = State::Ended;
            return None;
        }
        // Audio ran out first: keep time with the wall clock from here.
        if audio_done && session.wall.is_none() {
            session.wall = Some((Instant::now(), clock));
        }
        Some(match &session.next {
            Some(frame) => Duration::from_millis(frame.pts_ms.saturating_sub(clock).clamp(1, 40)),
            None => Duration::from_millis(10),
        })
    }
}

impl Session {
    fn start(player: &Player, start_ms: u64, play: bool) -> Result<Self, String> {
        let [width, height] = player.size;
        let seek = format!("{start_ms}ms");
        let mut video = crate::ffmpeg::command(&player.binary)
            .args(["-loglevel", "error", "-ss", &seek, "-i", &player.input])
            .args(["-map", "0:v:0", "-an", "-sn"])
            .args([
                "-vf",
                &format!("scale={width}:{height}:flags=bilinear,format=rgba"),
            ])
            .args(["-r", &format!("{}", player.fps), "-fps_mode", "cfr"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Could not start FFmpeg: {error}"))?;
        let stdout = video.stdout.take().ok_or("FFmpeg has no output")?;
        let (sender, frames) = mpsc::sync_channel(VIDEO_FRAMES);
        let frame_bytes = width * height * 4;
        let interval = 1000.0 / f64::from(player.fps);
        std::thread::spawn(move || read_frames(stdout, frame_bytes, start_ms, interval, sender));
        let audio = Audio::start(player, &seek, play);
        Ok(Self {
            start_ms,
            frames,
            next: None,
            video_done: false,
            video: Arc::new(Mutex::new(Some(video))),
            audio,
            wall: None,
            shown_any: false,
        })
    }

    fn clock_ms(&self) -> u64 {
        if let Some((since, from)) = self.wall {
            return from + since.elapsed().as_millis() as u64;
        }
        match &self.audio {
            Some(audio) => self.start_ms + audio.played_ms(),
            None => self.start_ms,
        }
    }

    /// The player keeps the paused position; a wall clock simply stops.
    fn pause(&mut self) {
        if let Some(audio) = &self.audio {
            audio.command(AudioCommand::Pause);
        }
        self.wall = None;
    }

    fn resume(&mut self, at: u64) {
        match &self.audio {
            Some(audio) if !audio.finished() => audio.command(AudioCommand::Play),
            _ => self.wall = Some((Instant::now(), at)),
        }
    }
}

fn read_frames(
    mut stdout: impl Read,
    frame_bytes: usize,
    start_ms: u64,
    interval: f64,
    sender: SyncSender<Frame>,
) {
    for index in 0_u64.. {
        let mut pixels = vec![0; frame_bytes];
        if stdout.read_exact(&mut pixels).is_err() {
            return;
        }
        let pts_ms = start_ms + (index as f64 * interval).round() as u64;
        if sender.send(Frame { pts_ms, pixels }).is_err() {
            return;
        }
    }
}

enum AudioCommand {
    Play,
    Pause,
    Volume(f32),
}

/// The audio decoder plus a thread that owns the output device.
struct Audio {
    commands: mpsc::Sender<AudioCommand>,
    played: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Drop for Audio {
    fn drop(&mut self) {
        if let Some(mut child) = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        // Dropping `commands` ends the output thread.
    }
}

impl Audio {
    /// `None` when the decoder cannot start; a video without sound starts
    /// fine and finishes at once.
    fn start(player: &Player, seek: &str, play: bool) -> Option<Self> {
        let mut child = crate::ffmpeg::command(&player.binary)
            .args(["-loglevel", "error", "-ss", seek, "-i", &player.input])
            .args(["-map", "0:a:0?", "-vn", "-sn", "-ac", "2", "-ar", "48000"])
            .args(["-c:a", "pcm_f32le", "-f", "f32le", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let (chunks_sender, chunks) = mpsc::sync_channel::<Vec<f32>>(AUDIO_CHUNKS);
        std::thread::spawn(move || read_pcm(stdout, chunks_sender));
        let played = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let (commands, receiver) = mpsc::channel();
        let source = PcmSource {
            chunks,
            chunk: Vec::new(),
            at: 0,
            played: played.clone(),
            finished: finished.clone(),
        };
        let volume = if player.muted { 0.0 } else { player.volume };
        std::thread::spawn(move || output(source, receiver, volume, play));
        Some(Self {
            commands,
            played,
            finished,
            child: Arc::new(Mutex::new(Some(child))),
        })
    }

    fn command(&self, command: AudioCommand) {
        let _ = self.commands.send(command);
    }

    fn played_ms(&self) -> u64 {
        self.played.load(Ordering::Relaxed) * 1000 / (u64::from(SAMPLE_RATE) * u64::from(CHANNELS))
    }

    fn finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
}

/// Plays `source` on the default device until the commands sender drops.
fn output(source: PcmSource, commands: mpsc::Receiver<AudioCommand>, volume: f32, play: bool) {
    let finished = source.finished.clone();
    let Ok(stream) = OutputStreamBuilder::open_default_stream() else {
        // No output device: the wall clock takes over.
        finished.store(true, Ordering::Relaxed);
        return;
    };
    let sink = Sink::connect_new(stream.mixer());
    sink.set_volume(volume);
    if !play {
        sink.pause();
    }
    sink.append(source);
    while let Ok(command) = commands.recv() {
        match command {
            AudioCommand::Play => sink.play(),
            AudioCommand::Pause => sink.pause(),
            AudioCommand::Volume(volume) => sink.set_volume(volume),
        }
    }
    sink.stop();
}

fn read_pcm(mut stdout: impl Read, sender: SyncSender<Vec<f32>>) {
    let mut bytes = vec![0_u8; CHUNK_SAMPLES * 4];
    loop {
        let mut filled = 0;
        while filled < bytes.len() {
            match stdout.read(&mut bytes[filled..]) {
                Ok(0) | Err(_) => break,
                Ok(read) => filled += read,
            }
        }
        // Whole stereo frames only.
        let usable = filled / 8 * 8;
        if usable > 0 {
            let samples = bytes[..usable]
                .chunks_exact(4)
                .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
                .collect();
            if sender.send(samples).is_err() {
                return;
            }
        }
        if filled < bytes.len() {
            return;
        }
    }
}

/// Decoded PCM for rodio. Underruns play silence without advancing the
/// clock, so video waits for audio instead of drifting ahead.
struct PcmSource {
    chunks: Receiver<Vec<f32>>,
    chunk: Vec<f32>,
    at: usize,
    played: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
}

impl Iterator for PcmSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.at >= self.chunk.len() {
            match self.chunks.try_recv() {
                Ok(chunk) => {
                    self.chunk = chunk;
                    self.at = 0;
                }
                Err(TryRecvError::Empty) => return Some(0.0),
                Err(TryRecvError::Disconnected) => {
                    self.finished.store(true, Ordering::Relaxed);
                    return None;
                }
            }
        }
        let sample = self.chunk.get(self.at).copied()?;
        self.at += 1;
        self.played.fetch_add(1, Ordering::Relaxed);
        Some(sample)
    }
}

impl Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        CHANNELS
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// The largest even size within `fit` (and the video's own size) that keeps
/// the aspect ratio.
pub fn output_size(width: u32, height: u32, fit: egui::Vec2) -> [usize; 2] {
    let (width, height) = (width.max(2) as f32, height.max(2) as f32);
    let scale = (fit.x / width).min(fit.y / height).clamp(0.01, 1.0);
    let even = |value: f32| ((value * scale / 2.0).round() as usize * 2).max(2);
    [even(width), even(height)]
}

/// "1:05" / "1:02:03".
pub fn time_label(ms: u64) -> String {
    let seconds = ms / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fit_the_viewer_without_enlarging_and_stay_even() {
        assert_eq!(
            output_size(3840, 2160, egui::vec2(1280.0, 800.0)),
            [1280, 720]
        );
        assert_eq!(
            output_size(1080, 1920, egui::vec2(1280.0, 800.0)),
            [450, 800]
        );
        assert_eq!(
            output_size(640, 360, egui::vec2(1920.0, 1080.0)),
            [640, 360]
        );
        assert_eq!(
            output_size(641, 361, egui::vec2(1920.0, 1080.0)),
            [642, 362]
        );
    }

    #[test]
    fn times_read_like_other_players() {
        assert_eq!(time_label(0), "0:00");
        assert_eq!(time_label(65_400), "1:05");
        assert_eq!(time_label(3_723_000), "1:02:03");
    }

    #[test]
    fn pcm_underruns_are_silent_and_do_not_advance_the_clock() {
        let (sender, chunks) = mpsc::sync_channel(4);
        let played = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let mut source = PcmSource {
            chunks,
            chunk: Vec::new(),
            at: 0,
            played: played.clone(),
            finished: finished.clone(),
        };
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(played.load(Ordering::Relaxed), 0);
        sender.send(vec![0.5, -0.5]).unwrap();
        assert_eq!(source.next(), Some(0.5));
        assert_eq!(source.next(), Some(-0.5));
        assert_eq!(played.load(Ordering::Relaxed), 2);
        drop(sender);
        assert_eq!(source.next(), None);
        assert!(finished.load(Ordering::Relaxed));
    }

    /// Real decoding (`CAPER_FFMPEG`): frames arrive at the clip's size and
    /// rate, a seek starts at the new time, and the end is reached.
    #[test]
    #[ignore = "needs an FFmpeg with lavfi and libx264 via CAPER_FFMPEG"]
    fn decodes_frames_seeks_and_ends_with_a_real_ffmpeg() {
        let binary = crate::ffmpeg::binary().expect("set CAPER_FFMPEG");
        let path = std::env::temp_dir().join(format!("caper-player-{}.mp4", uuid::Uuid::new_v4()));
        let status = crate::ffmpeg::command(binary)
            .args([
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=30",
            ])
            .args(["-t", "1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let context = egui::Context::default();
        let mut player = Player::new(
            binary,
            path.to_string_lossy().into_owned(),
            320,
            240,
            Some(1000),
            Some(30.0),
            egui::vec2(160.0, 160.0),
        );
        assert_eq!(player.frame_size(), egui::vec2(160.0, 120.0));
        // Paused at a time: decodes and shows just that frame.
        player.seek(500, false);
        let deadline = Instant::now() + Duration::from_secs(10);
        while player.texture().is_none() && Instant::now() < deadline {
            player.update(&context);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(player.state(), State::Paused);
        assert_eq!(player.texture().unwrap().size(), [160, 120]);
        assert_eq!(player.position_ms(), 500);
        // Playing (wall clock: no audio stream) reaches the end.
        player.play();
        let deadline = Instant::now() + Duration::from_secs(10);
        while player.state() != State::Ended && Instant::now() < deadline {
            player.update(&context);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(player.state(), State::Ended);
        assert_eq!(player.position_ms(), 1000);
        std::fs::remove_file(path).unwrap();
    }
}
