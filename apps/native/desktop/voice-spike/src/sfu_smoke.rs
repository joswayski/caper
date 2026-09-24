//! Explicitly authorized, bounded General speech acceptance. Never run in CI.

use super::*;
use libwebrtc::audio_stream::native::NativeAudioStream;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicU64;

const PRIVATE_PULSE: &str = "unix:/tmp/caper-voice-silent-parity/native";

#[derive(Deserialize)]
struct Presence {
    participants: Vec<PresentParticipant>,
}

#[derive(Deserialize)]
struct PresentParticipant {
    id: String,
}

fn only_owned(presence: &Presence, owned: &[(String, String)]) -> bool {
    presence
        .participants
        .iter()
        .all(|p| owned.iter().any(|(id, _)| *id == p.id))
}

async fn presence(api: &MediaApi) -> Result<Presence, String> {
    api.client
        .get(api.root.join("presence").unwrap())
        .send()
        .await
        .map_err(|_| "presence request failed")?
        .error_for_status()
        .map_err(|_| "presence request rejected")?
        .json()
        .await
        .map_err(|_| "invalid presence response".into())
}

fn private_pulse(args: &[&str]) -> String {
    let output = Command::new("pactl").args(args).output().unwrap();
    assert!(output.status.success(), "private PulseAudio command failed");
    String::from_utf8(output.stdout).unwrap()
}

struct Speech {
    child: Child,
    writer: Option<std::thread::JoinHandle<()>>,
}

impl Speech {
    fn start(pcm: Vec<u8>) -> Result<Self, String> {
        let mut child = Command::new("pacat")
            .args([
                "--playback",
                "--device=caper_silent_sink",
                "--raw",
                "--rate=48000",
                "--channels=1",
                "--format=s16le",
                "--latency-msec=20",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "could not start private speech injection")?;
        let mut input = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || {
            // The finite fixture bounds injection independently of networking.
            let _ = input.write_all(&pcm);
        });
        Ok(Self {
            child,
            writer: Some(writer),
        })
    }
}

impl Drop for Speech {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

struct Probe {
    peak: Arc<AtomicU64>,
    frames: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl Probe {
    fn new(track: MediaStreamTrack) -> Result<Self, String> {
        let MediaStreamTrack::Audio(track) = track else {
            return Err("expected audio track".into());
        };
        let peak = Arc::new(AtomicU64::new(0));
        let frames = Arc::new(AtomicU64::new(0));
        let (observed_peak, observed_frames) = (peak.clone(), frames.clone());
        let mut stream = NativeAudioStream::new(track, 48_000, 1);
        let task = tokio::spawn(async move {
            while let Some(frame) = stream.next_frame().await {
                observed_frames.fetch_add(1, Ordering::Relaxed);
                let peak = frame
                    .data
                    .iter()
                    .map(|v| u64::from(v.unsigned_abs()))
                    .max()
                    .unwrap_or(0);
                observed_peak.fetch_max(peak, Ordering::Relaxed);
            }
        });
        Ok(Self { peak, frames, task })
    }

    fn audible(&self) -> bool {
        self.peak.load(Ordering::Relaxed) > 500
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn publication_probe_observes_actual_source_pcm() {
    let factory = PeerConnectionFactory::default();
    let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
    let track = factory.create_audio_track("local-probe", source.clone());
    let probe = Probe::new(track.into()).unwrap();
    let zero = AudioFrame::new(48_000, 1, 480);
    source.capture_frame(&zero).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!probe.audible());
    let signal = AudioFrame {
        data: vec![-6300; 480].into(),
        sample_rate: 48_000,
        num_channels: 1,
        samples_per_channel: 480,
    };
    source.capture_frame(&signal).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !probe.audible() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(probe.peak.load(Ordering::Relaxed), 6300);
    assert!(probe.frames.load(Ordering::Relaxed) >= 2);
}

#[test]
fn owned_roster_rejects_strangers_even_when_owned_clients_are_present() {
    let owned = vec![
        ("owned-a".into(), "unused".into()),
        ("owned-b".into(), "unused".into()),
    ];
    let roster = |ids: &[&str]| Presence {
        participants: ids
            .iter()
            .map(|id| PresentParticipant { id: (*id).into() })
            .collect(),
    };
    assert!(only_owned(&roster(&[]), &owned));
    assert!(only_owned(&roster(&["owned-b", "owned-a"]), &owned));
    assert!(!only_owned(
        &roster(&["owned-a", "stranger", "owned-b"]),
        &owned
    ));
    assert!(!only_owned(&roster(&["owned-a"]), &[]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit public speech authorization and isolated private PulseAudio"]
async fn generated_speech_public_sfu_two_native_sessions() {
    assert_eq!(
        std::env::var("CAPER_SPEECH_SFU_SMOKE").as_deref(),
        Ok("authorized-20s")
    );
    assert_eq!(std::env::var("PULSE_SERVER").as_deref(), Ok(PRIVATE_PULSE));
    let sinks = private_pulse(&["list", "short", "sinks"]);
    assert_eq!(
        sinks.lines().count(),
        2,
        "refuse non-isolated output devices"
    );
    assert!(sinks.lines().all(|s| s.contains("module-null-sink.c")));
    let sources = private_pulse(&["list", "short", "sources"]);
    assert_eq!(sources.lines().count(), 2, "refuse physical input devices");
    assert!(sources.lines().all(|s| s.contains(".monitor\t")));
    private_pulse(&["set-default-source", "caper_silent_sink.monitor"]);
    // Received playback must never feed the capture monitor.
    private_pulse(&["set-default-sink", "caper_second_sink"]);
    let speech = Command::new("ffmpeg")
        .args([
            "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
            "flite=text=Caper automated voice verification using generated speech on an isolated virtual microphone. Testing clear speech with different syllables and changing cadence. This is a short bounded verification of native audio publication and remote reception.",
            "-t", "20", "-af", "apad", "-f", "s16le", "-ar", "48000", "-ac", "1", "pipe:1",
        ])
        .output().unwrap();
    assert!(speech.status.success());
    assert!(
        speech
            .stdout
            .chunks_exact(2)
            .any(|v| i16::from_le_bytes([v[0], v[1]]).unsigned_abs() > 500)
    );

    let api = MediaApi::new(&Url::parse("https://caper.chat/").unwrap(), None, None).unwrap();
    assert!(
        presence(&api).await.unwrap().participants.is_empty(),
        "General is occupied: no join or audio permitted"
    );
    let mut owned = Vec::new();
    let mut sessions = Vec::new();
    let mut publication = Vec::new();
    let mut received = Vec::new();
    let controls = [JoinControl::new(), JoinControl::new()];
    let mut injected = None;
    let work = async {
        for (index, control) in controls.iter().enumerate() {
            if !only_owned(&presence(&api).await?, &owned) {
                return Err("General became occupied".into());
            }
            let joined: JoinResponse = api.post("join", None, json!({
                "name": if index == 0 { "Caper Speech Verification A" } else { "Caper Speech Verification B" },
                "muted": true, "deafened": false,
            })).await.map_err(|_| "join request failed")?;
            // Retain every returned capability before fallible native setup.
            owned.push((joined.id.clone(), joined.token.clone()));
            let mut intent = AudioIntent::default();
            intent.set_muted(true);
            let session =
                NativeSession::from_join(api.clone(), joined, intent, None, None, control)
                    .await
                    .map_err(|_| "native transport setup failed")?;
            sessions.push(session);
            eprintln!("speech SFU: transport {} ready (still muted)", index + 1);
        }
        for session in &mut sessions {
            let allowed: Vec<_> = owned.iter().map(|(id, _)| id.as_str()).collect();
            session
                .snapshot_owned(&allowed)
                .await
                .map_err(|_| "owned-only subscription failed")?;
            publication.push(Probe::new(session.microphone.clone())?);
            let tracks: Vec<_> = session
                .local_control
                .playback
                .lock()
                .map_err(|_| "playback observation lock failed")?
                .tracks
                .values()
                .cloned()
                .collect();
            if tracks.len() != 1 {
                return Err("expected one owned remote audio track".into());
            }
            received.push(Probe::new(tracks[0].clone())?);
        }
        if !only_owned(&presence(&api).await?, &owned) {
            return Err("General became occupied".into());
        }
        for (session, control) in sessions.iter_mut().zip(&controls) {
            control.set_input_processing(175, 25)?;
            control.activate()?;
            session
                .set_muted(false)
                .await
                .map_err(|_| "unmute failed")?;
        }
        injected = Some(Speech::start(speech.stdout)?);
        let mut sampled = Instant::now();
        loop {
            // Withhold speech immediately on roster uncertainty; never subscribe
            // to unrelated audio and never retry an unsuccessful public test.
            let roster = tokio::time::timeout(Duration::from_millis(750), presence(&api))
                .await
                .map_err(|_| "roster watch timed out")??;
            if !only_owned(&roster, &owned) {
                return Err("unrelated participant appeared".into());
            }
            if publication.iter().all(Probe::audible) && received.iter().all(Probe::audible) {
                return Ok::<_, String>(());
            }
            if sampled.elapsed() >= Duration::from_secs(2) {
                for (index, session) in sessions.iter_mut().enumerate() {
                    let stats = session
                        .diagnostics()
                        .await
                        .map_err(|_| "RTC statistics failed")?;
                    eprintln!(
                        "speech SFU client {}: sent/received bytes {}/{}, publication/decoded peaks {}/{}",
                        index + 1,
                        stats.sent_bytes,
                        stats.received_bytes,
                        publication[index].peak.load(Ordering::Relaxed),
                        received[index].peak.load(Ordering::Relaxed)
                    );
                }
                sampled = Instant::now();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(20), work).await;
    // All exits (including setup failures and timeout) close local devices
    // before assertions/network cleanup.
    for control in &controls {
        control.cancel();
    }
    injected.take();
    for session in &mut sessions {
        session.close_local();
        session.token.clear();
    }
    for (index, probe) in publication.iter().enumerate() {
        eprintln!(
            "speech SFU publication {}: {} frames, peak {}",
            index + 1,
            probe.frames.load(Ordering::Relaxed),
            probe.peak.load(Ordering::Relaxed)
        );
    }
    for (index, probe) in received.iter().enumerate() {
        eprintln!(
            "speech SFU decoded {}: {} frames, peak {}",
            index + 1,
            probe.frames.load(Ordering::Relaxed),
            probe.peak.load(Ordering::Relaxed)
        );
    }
    let mut cleanup_ok = true;
    for (_, token) in &owned {
        let leave = tokio::time::timeout(
            Duration::from_secs(5),
            api.post_empty("leave", token, json!({})),
        )
        .await;
        cleanup_ok &= matches!(leave, Ok(Ok(())));
    }
    let roster = presence(&api).await.unwrap();
    let absent = roster
        .participants
        .iter()
        .all(|p| owned.iter().all(|(id, _)| *id != p.id));
    eprintln!(
        "speech SFU cleanup: leave acknowledged={cleanup_ok}, owned participants absent={absent}"
    );
    assert!(
        absent,
        "owned test participants still present after cleanup"
    );
    assert!(
        cleanup_ok,
        "remote leave did not acknowledge every owned capability"
    );
    assert!(
        matches!(result, Ok(Ok(()))),
        "bounded speech test failed: {result:?}"
    );
}
