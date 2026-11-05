//! SCRATCH benchmark probe, never committed: one line per App::update call
//! (wall-clock micros since start, update duration in micros).
use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;

static OUT: Mutex<Option<(Instant, std::io::BufWriter<std::fs::File>)>> = Mutex::new(None);

pub struct Probe(Instant);

impl Probe {
    pub fn start() -> Self {
        Self(Instant::now())
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let mut out = OUT.lock().unwrap();
        if out.is_none() {
            let Ok(path) = std::env::var("CAPER_BENCH_FRAMES") else {
                return;
            };
            *out = Some((
                Instant::now(),
                std::io::BufWriter::new(std::fs::File::create(path).unwrap()),
            ));
        }
        if let Some((_epoch, file)) = out.as_mut() {
            let now = Instant::now();
            let _ = writeln!(
                file,
                "{} {}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_micros(),
                now.duration_since(self.0).as_micros()
            );
            let _ = file.flush();
        }
    }
}
