//! Times the provider calls on a Caper join's critical path against any
//! Cloudflare-Realtime-compatible API: Cloudflare itself or caper-sfu. Run it
//! where the Caper API runs (inside the cluster) so the numbers match
//! production. Reads the API's own settings:
//!
//! `MEDIA_PROVIDER_BASE` (default Cloudflare), `CF_SFU_APP_ID`,
//! `CF_SFU_APP_SECRET`, `CF_TURN_KEY_ID`, `CF_TURN_API_TOKEN`, and optionally
//! `BENCH_RUNS` (default 10). It creates short-lived sessions that never
//! connect, publishes no audio, closes what it opened and revokes its TURN
//! credentials. It prints no secrets or SDP.

use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::{
    env,
    time::{Duration, Instant},
};
use str0m::{
    Rtc,
    change::{SdpAnswer, SdpOffer},
    media::{Direction, MediaKind},
};

const CLOUDFLARE: &str = "https://rtc.live.cloudflare.com/v1";

struct Provider {
    client: Client,
    base: String,
    app: String,
    secret: String,
    turn_key: String,
    turn_token: String,
}

impl Provider {
    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("{}/apps/{}/{path}", self.base, self.app);
        let mut request = self.client.request(method, url).bearer_auth(&self.secret);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        let status = response.status();
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(value)
        } else {
            Err(format!("{path}: HTTP {status} {}", value["errorCode"]))
        }
    }

    async fn turn(&self) -> Result<Option<String>, String> {
        let response = self
            .client
            .post(format!(
                "{}/turn/keys/{}/credentials/generate-ice-servers",
                self.base, self.turn_key
            ))
            .bearer_auth(&self.turn_token)
            .json(&json!({"ttl": 3600}))
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        if !response.status().is_success() {
            return Err(format!("turn: HTTP {}", response.status()));
        }
        let value: Value = response.json().await.map_err(|e| e.to_string())?;
        Ok(value["iceServers"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|s| s["username"].as_str().map(str::to_owned)))
    }

    async fn revoke(&self, username: &str) {
        let _ = self
            .client
            .post(format!(
                "{}/turn/keys/{}/credentials/{username}/revoke",
                self.base, self.turn_key
            ))
            .bearer_auth(&self.turn_token)
            .send()
            .await;
    }
}

fn required(key: &str) -> String {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| panic!("{key} is required"))
}

#[derive(Default)]
struct Samples {
    create: Vec<Duration>,
    turn: Vec<Duration>,
    provisioning: Vec<Duration>,
    publish: Vec<Duration>,
    pull: Vec<Duration>,
    publication: Vec<Duration>,
    negotiate: Vec<Duration>,
}

fn stats(label: &str, samples: &[Duration]) {
    let mut ms = samples
        .iter()
        .map(|d| d.as_secs_f64() * 1000.0)
        .collect::<Vec<_>>();
    if ms.is_empty() {
        return;
    }
    ms.sort_by(f64::total_cmp);
    let at = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    println!(
        "{label:<44} median {:>6.1} ms   p90 {:>6.1} ms   min {:>6.1}   max {:>6.1}",
        at(0.5),
        at(0.9),
        ms[0],
        ms[ms.len() - 1]
    );
}

#[tokio::main]
async fn main() -> Result<(), String> {
    str0m::crypto::from_feature_flags().install_process_default();
    let provider = Provider {
        client: Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?,
        base: env::var("MEDIA_PROVIDER_BASE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| CLOUDFLARE.into())
            .trim_end_matches('/')
            .to_owned(),
        app: required("CF_SFU_APP_ID"),
        secret: required("CF_SFU_APP_SECRET"),
        turn_key: required("CF_TURN_KEY_ID"),
        turn_token: required("CF_TURN_API_TOKEN"),
    };
    let runs: usize = env::var("BENCH_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    println!(
        "Benchmarking {} ({runs} runs after one warm-up)",
        provider.base
    );

    let mut samples = Samples::default();
    // The previous run's published track stands in for someone already in the room.
    let mut previous: Option<(String, String)> = None;
    for run in 0..=runs {
        // Round 1 (skipped for signed-in joins): sessions and TURN, in parallel.
        let started = Instant::now();
        let create = || async {
            let started = Instant::now();
            let value = provider.call(Method::POST, "sessions/new", None).await;
            (value, started.elapsed())
        };
        let ((main, main_time), (receive, _), (turn, turn_time)) =
            tokio::join!(create(), create(), async {
                let started = Instant::now();
                (provider.turn().await, started.elapsed())
            });
        let provisioning = started.elapsed();
        let main = main?["sessionId"]
            .as_str()
            .ok_or("no sessionId")?
            .to_owned();
        let receive = receive?["sessionId"]
            .as_str()
            .ok_or("no sessionId")?
            .to_owned();
        let username = turn?;

        // Round 2: publish the microphone and pull everyone present, in parallel.
        let mut sender = Rtc::new(Instant::now());
        let mut change = sender.sdp_api();
        let mid = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let (offer, pending) = change.apply().ok_or("no offer")?;
        let track = format!("bench-{run}");
        let round2 = Instant::now();
        let publishing = async {
            let started = Instant::now();
            let value = provider
                .call(
                    Method::POST,
                    &format!("sessions/{main}/tracks/new"),
                    Some(json!({
                        "sessionDescription": {"type": "offer", "sdp": offer.to_sdp_string()},
                        "tracks": [{"location": "local", "mid": mid.to_string(), "trackName": track}],
                    })),
                )
                .await;
            (value, started.elapsed())
        };
        let (source_session, source_track) = previous
            .clone()
            .unwrap_or_else(|| (main.clone(), "absent".into()));
        let pulling = async {
            let started = Instant::now();
            let value = provider
                .call(
                    Method::POST,
                    &format!("sessions/{receive}/tracks/new"),
                    Some(json!({"tracks": [{"location": "remote", "sessionId": source_session, "trackName": source_track}]})),
                )
                .await;
            (value, started.elapsed())
        };
        let ((published, publish_time), (pulled, pull_time)) = tokio::join!(publishing, pulling);
        let publication = round2.elapsed();
        let published = published?;
        let answer = SdpAnswer::from_sdp_string(
            published["sessionDescription"]["sdp"]
                .as_str()
                .ok_or("no answer")?,
        )
        .map_err(|e| e.to_string())?;
        sender
            .sdp_api()
            .accept_answer(pending, answer)
            .map_err(|e| e.to_string())?;

        // After join: answer the receive offer (gates hearing others).
        let pulled = pulled?;
        let mut negotiate_time = None;
        if let Some(sdp) = pulled["sessionDescription"]["sdp"].as_str() {
            let mut receiver = Rtc::new(Instant::now());
            let offer = SdpOffer::from_sdp_string(sdp).map_err(|e| e.to_string())?;
            let answer = receiver
                .sdp_api()
                .accept_offer(offer)
                .map_err(|e| e.to_string())?;
            let started = Instant::now();
            provider
                .call(
                    Method::PUT,
                    &format!("sessions/{receive}/renegotiate"),
                    Some(json!({"sessionDescription": {"type": "answer", "sdp": answer.to_sdp_string()}})),
                )
                .await?;
            negotiate_time = Some(started.elapsed());
        }

        // Clean up: close what was opened, revoke TURN.
        let _ = provider
            .call(
                Method::PUT,
                &format!("sessions/{main}/tracks/close"),
                Some(json!({"tracks": [{"mid": mid.to_string()}], "force": true})),
            )
            .await;
        if let Some(username) = &username {
            provider.revoke(username).await;
        }
        previous = Some((main, track));

        if run == 0 {
            continue; // Warm-up: includes TLS setup the API has already paid.
        }
        samples.create.push(main_time);
        samples.turn.push(turn_time);
        samples.provisioning.push(provisioning);
        samples.publish.push(publish_time);
        samples.pull.push(pull_time);
        samples.publication.push(publication);
        samples.negotiate.extend(negotiate_time);
    }

    println!();
    stats("create session", &samples.create);
    stats("TURN credentials", &samples.turn);
    stats("round 1: sessions + TURN (parallel)", &samples.provisioning);
    stats("publish microphone", &samples.publish);
    stats("pull present tracks", &samples.pull);
    stats("round 2: publish + pull (parallel)", &samples.publication);
    stats("renegotiate (gates hearing others)", &samples.negotiate);
    let cold = samples
        .provisioning
        .iter()
        .zip(&samples.publication)
        .map(|(a, b)| *a + *b)
        .collect::<Vec<_>>();
    println!();
    stats(
        "provider time, signed-in join (round 2)",
        &samples.publication,
    );
    stats("provider time, guest join (rounds 1 + 2)", &cold);
    Ok(())
}
