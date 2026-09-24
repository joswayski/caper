//! Native DPDFNet-8 inference. The model and initialization metadata are the
//! exact assets shipped by the web client; each capture owns a separate state.

use super::dpdfnet_dsp::{DpdfnetStream, HOP};
use ort::{session::Session, value::Tensor};
use serde::Deserialize;

const MODEL: &[u8] =
    include_bytes!("../../../../web/public/audio/dpdfnet8-v2/dpdfnet8_48khz_hr.onnx");
const METADATA: &str = include_str!("../../../../web/public/audio/dpdfnet8-v2/metadata.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    state_size: usize,
    erb_norm_state_size: usize,
    erb_norm_init: Vec<f32>,
    spec_norm_init: Vec<f32>,
}

fn initial_state() -> Result<Vec<f32>, String> {
    let metadata: Metadata = serde_json::from_str(METADATA).map_err(|e| e.to_string())?;
    if metadata.state_size != 90228
        || metadata.erb_norm_state_size != metadata.erb_norm_init.len()
        || metadata.erb_norm_init.len() + metadata.spec_norm_init.len() > metadata.state_size
    {
        return Err("invalid denoiser metadata".into());
    }
    let mut state = vec![0.0; metadata.state_size];
    state[..metadata.erb_norm_init.len()].copy_from_slice(&metadata.erb_norm_init);
    let offset = metadata.erb_norm_state_size;
    state[offset..offset + metadata.spec_norm_init.len()].copy_from_slice(&metadata.spec_norm_init);
    Ok(state)
}

pub struct Dpdfnet {
    session: Session,
    stream: DpdfnetStream,
}

impl Dpdfnet {
    pub fn new() -> Result<Self, String> {
        // Tests may specify a pinned runtime path. Packaged builds put the
        // verified native library beside the executable (or in lib/ on Linux).
        let path = std::env::var_os("CAPER_ONNXRUNTIME_LIBRARY")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let exe = std::env::current_exe().unwrap_or_default();
                let name = if cfg!(windows) {
                    "onnxruntime.dll"
                } else {
                    "libonnxruntime.so.1.23.2"
                };
                let adjacent = exe
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(name);
                if adjacent.is_file() || cfg!(windows) {
                    adjacent
                } else {
                    std::path::PathBuf::from("/usr/lib/caper-desktop").join(name)
                }
            });
        ort::init_from(path).map_err(|e| e.to_string())?.commit();
        let mut session = Session::builder()
            .map_err(|e| e.to_string())?
            .with_intra_threads(1)
            .map_err(|e| e.to_string())?
            .commit_from_memory(MODEL)
            .map_err(|e| e.to_string())?;
        let state = initial_state()?;
        // Warm up the graph without carrying its recurrent state into capture.
        Self::infer(&mut session, &vec![0.0; 962], &state)?;
        Ok(Self {
            session,
            stream: DpdfnetStream::new(state),
        })
    }

    fn infer(
        session: &mut Session,
        spec: &[f32],
        state: &[f32],
    ) -> Result<(Vec<f32>, Vec<f32>), String> {
        let spec =
            Tensor::from_array(([1usize, 1, 481, 2], spec.to_vec())).map_err(|e| e.to_string())?;
        let state =
            Tensor::from_array(([90228usize], state.to_vec())).map_err(|e| e.to_string())?;
        let outputs = session
            .run(ort::inputs!["spec" => spec, "state_in" => state])
            .map_err(|e| e.to_string())?;
        let (_, spec) = outputs["spec_e"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let (_, state) = outputs["state_out"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        Ok((spec.to_vec(), state.to_vec()))
    }

    pub fn process(&mut self, hop: &[f32; HOP]) -> Result<[f32; HOP], String> {
        self.stream.process(hop, |spec, state| {
            Self::infer(&mut self.session, spec, state)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_matches_web_metadata() {
        let state = initial_state().unwrap();
        assert_eq!(state.len(), 90228);
        assert!((state[0] + 32.557_3).abs() < 1e-4);
        assert!(state.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    #[ignore = "requires the pinned, locally installed ONNX Runtime library"]
    fn native_inference_is_finite_and_owns_fresh_state() {
        let mut model = Dpdfnet::new().unwrap();
        // Independently generated with Python onnxruntime==1.23.2 and the
        // shipped metadata/model: one zero spectrum with fresh recurrent state.
        let (_, next) = Dpdfnet::infer(
            &mut model.session,
            &vec![0.0; 962],
            &initial_state().unwrap(),
        )
        .unwrap();
        for (actual, reference) in
            next.iter()
                .zip([-33.906_155, -36.124_78, -40.076_527, -41.081_028])
        {
            assert!(
                (actual - reference).abs() < 0.0001,
                "native ORT differs from reference"
            );
        }
        for hop in 0..10 {
            let input = std::array::from_fn(|i| {
                let t = (hop * HOP + i) as f32 / 48000.0;
                (t * std::f32::consts::TAU * 311.0).sin() * 0.07
            });
            assert!(model.process(&input).unwrap().iter().all(|x| x.is_finite()));
        }
    }
}
