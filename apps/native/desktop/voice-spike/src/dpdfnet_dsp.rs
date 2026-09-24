//! 48 kHz DPDFNet streaming STFT/ISTFT, ported from the shipped web dsp.js.
//! Inference runs off the audio callback; each capture owns fresh state/OLA.

const N: usize = 960;
pub const HOP: usize = 480;
const BINS: usize = 481;

struct Plan {
    n: usize,
    radix: usize,
    size: usize,
    roots: Vec<f64>,
    output: Vec<f64>,
    children: Vec<Self>,
}

impl Plan {
    fn new(n: usize, inverse: bool) -> Self {
        if n == 1 {
            return Self {
                n,
                radix: 0,
                size: 0,
                roots: vec![],
                output: vec![0.0; 2],
                children: vec![],
            };
        }
        let radix = [2, 3, 5]
            .into_iter()
            .find(|value| n.is_multiple_of(*value))
            .unwrap_or(n);
        let size = n / radix;
        let mut roots = vec![0.0; n * radix * 2];
        for index in 0..n {
            for r in 0..radix {
                let sign = if inverse { 1.0 } else { -1.0 };
                let angle = sign * std::f64::consts::TAU * (r * index) as f64 / n as f64;
                let at = (index * radix + r) * 2;
                roots[at] = angle.cos();
                roots[at + 1] = angle.sin();
            }
        }
        Self {
            n,
            radix,
            size,
            roots,
            output: vec![0.0; n * 2],
            children: (0..radix).map(|_| Self::new(size, inverse)).collect(),
        }
    }

    fn execute(&mut self, input: &[f64], offset: usize, stride: usize) {
        if self.n == 1 {
            self.output
                .copy_from_slice(&input[offset * 2..offset * 2 + 2]);
            return;
        }
        for (r, child) in self.children.iter_mut().enumerate() {
            child.execute(input, offset + r * stride, stride * self.radix);
        }
        for q in 0..self.size {
            for s in 0..self.radix {
                let index = q + self.size * s;
                let (mut re, mut im) = (0.0, 0.0);
                for r in 0..self.radix {
                    let part = &self.children[r].output;
                    let (pr, pi) = (part[q * 2], part[q * 2 + 1]);
                    let at = (index * self.radix + r) * 2;
                    let (cr, ci) = (self.roots[at], self.roots[at + 1]);
                    re += pr * cr - pi * ci;
                    im += pr * ci + pi * cr;
                }
                self.output[index * 2] = re;
                self.output[index * 2 + 1] = im;
            }
        }
    }
}

pub struct DpdfnetStream {
    forward: Plan,
    inverse: Plan,
    window: [f32; N],
    analysis: [f32; N],
    ola: [f32; N],
    time: Vec<f64>,
    spectrum: Vec<f64>,
    state: Vec<f32>,
    primed: bool,
}

impl DpdfnetStream {
    pub fn new(initial_state: Vec<f32>) -> Self {
        let window = std::array::from_fn(|i| {
            (std::f32::consts::FRAC_PI_2
                * (std::f32::consts::PI * (i as f32 + 0.5) / N as f32)
                    .sin()
                    .powi(2))
            .sin()
        });
        Self {
            forward: Plan::new(N, false),
            inverse: Plan::new(N, true),
            window,
            analysis: [0.0; N],
            ola: [0.0; N],
            time: vec![0.0; N * 2],
            spectrum: vec![0.0; N * 2],
            state: initial_state,
            primed: false,
        }
    }

    /// `run` performs one inference and returns the enhanced complex spectrum
    /// and next recurrent state. Invalid output fails closed, never bypasses.
    pub fn process(
        &mut self,
        hop: &[f32; HOP],
        run: impl FnOnce(&[f32], &[f32]) -> Result<(Vec<f32>, Vec<f32>), String>,
    ) -> Result<[f32; HOP], String> {
        self.analysis.copy_within(HOP.., 0);
        self.analysis[HOP..].copy_from_slice(hop);
        self.time.fill(0.0);
        for i in 0..N {
            self.time[i * 2] = f64::from(self.analysis[i] * self.window[i]);
        }
        self.forward.execute(&self.time, 0, 1);
        let spec: Vec<f32> = self.forward.output[..BINS * 2]
            .iter()
            .map(|value| *value as f32)
            .collect();
        let (enhanced, state) = run(&spec, &self.state)?;
        if enhanced.len() != BINS * 2
            || state.len() != self.state.len()
            || !enhanced.iter().chain(&state).all(|value| value.is_finite())
        {
            return Err("invalid denoiser output".into());
        }
        self.state = state;
        self.spectrum.fill(0.0);
        for (target, value) in self.spectrum.iter_mut().zip(&enhanced) {
            *target = f64::from(*value);
        }
        for i in 1..BINS - 1 {
            self.spectrum[(N - i) * 2] = f64::from(enhanced[i * 2]);
            self.spectrum[(N - i) * 2 + 1] = -f64::from(enhanced[i * 2 + 1]);
        }
        self.inverse.execute(&self.spectrum, 0, 1);
        let mut output = [0.0; HOP];
        for (i, sample) in output.iter_mut().enumerate() {
            *sample = self.ola[i] + (self.inverse.output[i * 2] / N as f64) as f32 * self.window[i];
        }
        self.ola.copy_within(HOP.., 0);
        self.ola[HOP..].fill(0.0);
        for i in 0..HOP {
            self.ola[i] +=
                (self.inverse.output[(i + HOP) * 2] / N as f64) as f32 * self.window[i + HOP];
        }
        if !self.primed {
            self.primed = true;
            output.fill(0.0);
        }
        if output.iter().all(|sample| sample.is_finite()) {
            Ok(output)
        } else {
            Err("invalid denoiser PCM".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_inference_matches_shipped_javascript_dsp_reference() {
        let mut stream = DpdfnetStream::new(vec![0.0; 90228]);
        let expected = [
            [0.0; 12],
            [
                0.03, 0.0338568, 0.03728378, 0.04028323, 0.04286336, 0.04503809, 0.04682673,
                0.04825369, 0.04934799, 0.05014279, 0.05067486, 0.05098395,
            ],
            [
                0.0868578, 0.08750173, 0.08774124, 0.08761024, 0.0871464, 0.08639069, 0.08538656,
                0.08417943, 0.08281593, 0.08134315, 0.079808, 0.07825644,
            ],
        ];
        for (hop, expected) in expected.into_iter().enumerate() {
            let input = std::array::from_fn(|i| {
                let time = (i + hop * HOP) as f64 / 48_000.0;
                (0.1 * (std::f64::consts::TAU * 311.0 * time).sin()
                    + 0.03 * (std::f64::consts::TAU * 911.0 * time).cos()) as f32
            });
            let out = stream
                .process(&input, |spec, state| Ok((spec.to_vec(), state.to_vec())))
                .unwrap();
            for (actual, expected) in out[..12].iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 0.00002,
                    "JS reference mismatch: {actual} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn invalid_inference_fails_closed() {
        let mut stream = DpdfnetStream::new(vec![0.0; 90228]);
        assert!(
            stream
                .process(&[0.0; HOP], |_, state| Ok((
                    vec![f32::NAN; BINS * 2],
                    state.to_vec()
                )))
                .is_err()
        );
    }
}
