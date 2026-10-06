//! `caper-media-worker process <input> <outdir> [--settings file.json]
//! [--filename name] [--content-type type] [--deadline seconds]`: runs only
//! the processing step and prints the body `POST …/finish` would send. The
//! stored result lands in `<outdir>/<filename>`, the preview in
//! `<outdir>/preview.webp`; preview/progress callbacks are logged to stderr.
//! `--deadline` simulates the Lambda time limit. Used by the integration
//! tests and for tuning settings by hand.

use std::path::{Path, PathBuf};

use std::time::{Duration, Instant};

use crate::process::{self, Ctx, EarlyPreview, Hooks, Job};
use crate::settings::Settings;

pub const USAGE: &str = "usage: caper-media-worker process <input> <outdir> [--settings settings.json] [--filename name] [--content-type type] [--deadline seconds]";

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub input: PathBuf,
    pub outdir: PathBuf,
    pub settings: Option<PathBuf>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub deadline: Option<Duration>,
}

/// Logs what the Lambda would send as `preview`/`progress` callbacks.
struct LogHooks;

#[async_trait::async_trait]
impl Hooks for LogHooks {
    async fn preview(&self, _: &Path, preview: &EarlyPreview) {
        tracing::info!(body = %serde_json::to_string(preview).unwrap_or_default(), "preview callback");
    }

    async fn progress(&self, percent: u8) {
        tracing::info!(percent, "progress callback");
    }
}

pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut positional = Vec::new();
    let (mut settings, mut filename, mut content_type, mut deadline) = (None, None, None, None);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--settings" => settings = Some(PathBuf::from(value()?)),
            "--filename" => filename = Some(value()?),
            "--content-type" => content_type = Some(value()?),
            "--deadline" => {
                let seconds: f64 = value()?.parse().map_err(|_| "--deadline needs seconds")?;
                if !(seconds.is_finite() && seconds > 0.0) {
                    return Err("--deadline needs positive seconds".into());
                }
                deadline = Some(Duration::from_secs_f64(seconds));
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => positional.push(PathBuf::from(other)),
        }
    }
    let [input, outdir] = <[PathBuf; 2]>::try_from(positional).map_err(|_| USAGE.to_owned())?;
    Ok(Args {
        input,
        outdir,
        settings,
        filename,
        content_type,
        deadline,
    })
}

fn copy_out(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::copy(from, to).map(|_| ())
}

pub async fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = parse_args(args)?;
    let settings = match &args.settings {
        Some(path) => Settings::from_value(&serde_json::from_str(&std::fs::read_to_string(path)?)?),
        None => Settings::default(),
    };
    let filename = args.filename.clone().unwrap_or_else(|| {
        args.input
            .file_name()
            .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned())
    });
    let job = Job {
        filename,
        declared_content_type: args.content_type.clone().unwrap_or_default(),
        settings,
    };
    std::fs::create_dir_all(&args.outdir)?;
    let work = args.outdir.join(format!(".work-{}", std::process::id()));
    std::fs::create_dir_all(&work)?;
    let ctx = Ctx {
        job: &job,
        work: &work,
        deadline: args.deadline.map(|d| Instant::now() + d),
        hooks: &LogHooks,
    };
    let result = process::process(&args.input, &ctx).await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(error.into());
        }
    };
    let stored = args.outdir.join(&outcome.finish.filename);
    let copied = copy_out(&outcome.result, &stored).and_then(|()| match &outcome.preview {
        Some(preview) => copy_out(preview, &args.outdir.join("preview.webp")),
        None => Ok(()),
    });
    let _ = std::fs::remove_dir_all(&work);
    copied?;
    println!("{}", serde_json::to_string_pretty(&outcome.finish)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn arguments() {
        let args = parse_args(&strings(&[
            "in.png",
            "out",
            "--settings",
            "s.json",
            "--filename",
            "a b.png",
        ]))
        .unwrap();
        assert_eq!(args.input, PathBuf::from("in.png"));
        assert_eq!(args.outdir, PathBuf::from("out"));
        assert_eq!(args.settings, Some(PathBuf::from("s.json")));
        assert_eq!(args.filename.as_deref(), Some("a b.png"));
        assert!(parse_args(&strings(&["in.png"])).is_err());
        assert!(parse_args(&strings(&["a", "b", "c"])).is_err());
        assert!(parse_args(&strings(&["a", "b", "--settings"])).is_err());
        assert!(parse_args(&strings(&["a", "b", "--nope"])).is_err());
        let args = parse_args(&strings(&["a", "b", "--deadline", "1.5"])).unwrap();
        assert_eq!(args.deadline, Some(Duration::from_millis(1500)));
        assert!(parse_args(&strings(&["a", "b", "--deadline", "-1"])).is_err());
    }
}
