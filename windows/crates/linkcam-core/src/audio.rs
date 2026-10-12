//! Streaming noise reduction: short-time Fourier transform with a spectral
//! gate. The noise floor is tracked as a slowly rising minimum of each
//! frequency bin's magnitude, then bins close to the floor are attenuated.
//!
//! Deliberately simple (no ML): it removes steady background noise such as
//! fans and hiss, which is what a webcam microphone mostly picks up.

use std::f32::consts::PI;

const FRAME: usize = 1024;
const HOP: usize = FRAME / 2;
/// How fast the tracked floor may rise per frame (slow, so speech is not "learned").
const FLOOR_RISE: f32 = 1.002;
/// Averaging of each bin's magnitude across frames before tracking the floor.
const MAGNITUDE_SMOOTHING: f32 = 0.9;
/// Smoothing of the gain over time, against musical noise.
const GAIN_SMOOTHING: f32 = 0.6;
/// Lowest gain the gate applies, so the result never sounds "dead".
const MIN_GAIN: f32 = 0.08;

/// In-place iterative radix-2 FFT of `re`/`im` (length must be a power of two).
fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= n {
        let angle = 2.0 * PI / length as f32 * if inverse { 1.0 } else { -1.0 };
        let (w_re, w_im) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(length) {
            let (mut cur_re, mut cur_im) = (1.0f32, 0.0f32);
            for k in 0..length / 2 {
                let (a, b) = (start + k, start + k + length / 2);
                let (t_re, t_im) = (re[b] * cur_re - im[b] * cur_im, re[b] * cur_im + im[b] * cur_re);
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                let next_re = cur_re * w_re - cur_im * w_im;
                cur_im = cur_re * w_im + cur_im * w_re;
                cur_re = next_re;
            }
        }
        length <<= 1;
    }
    if inverse {
        for (r, i) in re.iter_mut().zip(im.iter_mut()) {
            *r /= n as f32;
            *i /= n as f32;
        }
    }
}

/// One-channel noise reducer working on arbitrary-size chunks.
pub struct NoiseReducer {
    window: Vec<f32>,
    /// Samples received but not yet consumed by a full frame.
    pending: Vec<f32>,
    /// Overlap-add accumulator for the output.
    overlap: Vec<f32>,
    /// Per-bin magnitude averaged over a few frames (the floor follows this).
    smoothed: Vec<f32>,
    floor: Vec<f32>,
    gain: Vec<f32>,
    /// How strongly the floor is subtracted (1.0 mild .. 3.0 strong).
    strength: f32,
    primed: bool,
}

impl NoiseReducer {
    pub fn new(strength: f32) -> Self {
        let window = (0..FRAME).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FRAME as f32).cos()).collect();
        Self {
            window,
            pending: Vec::new(),
            overlap: vec![0.0; FRAME],
            smoothed: vec![0.0; FRAME / 2 + 1],
            floor: vec![f32::MAX; FRAME / 2 + 1],
            gain: vec![1.0; FRAME / 2 + 1],
            strength: strength.clamp(0.5, 4.0),
            primed: false,
        }
    }

    /// Feeds samples and returns the same number of cleaned samples, delayed by
    /// one frame of latency (the first `HOP` outputs are silence).
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.pending.extend_from_slice(input);
        let mut output = Vec::with_capacity(input.len());
        while self.pending.len() >= FRAME {
            let frame: Vec<f32> = self.pending[..FRAME].to_vec();
            self.pending.drain(..HOP);
            let cleaned = self.process_frame(&frame);
            for (acc, v) in self.overlap.iter_mut().zip(&cleaned) {
                *acc += v;
            }
            output.extend_from_slice(&self.overlap[..HOP]);
            self.overlap.drain(..HOP);
            self.overlap.extend(std::iter::repeat_n(0.0, HOP));
        }
        output
    }

    fn process_frame(&mut self, frame: &[f32]) -> Vec<f32> {
        let mut re: Vec<f32> = frame.iter().zip(&self.window).map(|(s, w)| s * w).collect();
        let mut im = vec![0.0f32; FRAME];
        fft(&mut re, &mut im, false);

        for bin in 0..=FRAME / 2 {
            let magnitude = (re[bin] * re[bin] + im[bin] * im[bin]).sqrt();
            // Track the floor on a smoothed magnitude: follow drops immediately,
            // rise slowly (so a word is not learned as noise).
            self.smoothed[bin] = if self.primed {
                self.smoothed[bin] * MAGNITUDE_SMOOTHING + magnitude * (1.0 - MAGNITUDE_SMOOTHING)
            } else {
                magnitude // start from the first observation, not from zero
            };
            let level = self.smoothed[bin];
            self.floor[bin] = if level < self.floor[bin] { level } else { self.floor[bin] * FLOOR_RISE };
            let target = if magnitude > 1e-9 {
                (1.0 - self.strength * self.floor[bin] / magnitude).clamp(MIN_GAIN, 1.0)
            } else {
                MIN_GAIN
            };
            self.gain[bin] = self.gain[bin] * GAIN_SMOOTHING + target * (1.0 - GAIN_SMOOTHING);
            re[bin] *= self.gain[bin];
            im[bin] *= self.gain[bin];
            if bin != 0 && bin != FRAME / 2 {
                re[FRAME - bin] = re[bin];
                im[FRAME - bin] = -im[bin];
            }
        }
        self.primed = true;
        fft(&mut re, &mut im, true);
        // Hann analysis window at 50% overlap sums to one, so no synthesis window is needed.
        re
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
    }

    /// Deterministic pseudo-random noise in -1..1.
    fn noise(len: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0
            })
            .collect()
    }

    #[test]
    fn fft_roundtrip_restores_the_signal() {
        let original: Vec<f32> = (0..64).map(|i| (i as f32 * 0.37).sin()).collect();
        let (mut re, mut im) = (original.clone(), vec![0.0; 64]);
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for (a, b) in original.iter().zip(&re) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn steady_noise_is_attenuated() {
        let mut reducer = NoiseReducer::new(2.0);
        let input: Vec<f32> = noise(48_000, 7).iter().map(|n| n * 0.05).collect();
        let output = reducer.process(&input);
        // Compare after the floor has settled (skip the first quarter second).
        let settled_in = rms(&input[12_000..]);
        let settled_out = rms(&output[12_000..]);
        assert!(settled_out < settled_in * 0.6, "noise {settled_in} -> {settled_out}");
    }

    #[test]
    fn tone_bursts_over_noise_keep_most_of_their_energy() {
        // Speech-like: 200 ms of tone, 300 ms of silence, over steady noise.
        let rate = 48_000usize;
        let tone: Vec<f32> = (0..rate * 3)
            .map(|i| {
                let on = (i % (rate / 2)) < rate / 5;
                if on {
                    0.5 * (2.0 * PI * 440.0 * i as f32 / rate as f32).sin()
                } else {
                    0.0
                }
            })
            .collect();
        let mixed: Vec<f32> = tone.iter().zip(noise(rate * 3, 3)).map(|(t, n)| t + n * 0.02).collect();
        let mut reducer = NoiseReducer::new(1.5);
        let output = reducer.process(&mixed);

        // Look at the last two bursts, allowing for the one-hop latency.
        let burst_energy = |signal: &[f32]| -> f32 {
            let mut total = 0.0;
            for burst in 3..6 {
                let start = burst * rate / 2 + HOP;
                total += rms(&signal[start..start + rate / 5 - HOP]);
            }
            total
        };
        let kept = burst_energy(&output);
        let original = burst_energy(&tone);
        assert!(kept > 0.5 * original, "burst energy {original} -> {kept}");
    }

    #[test]
    fn output_length_follows_input_in_whole_hops() {
        let mut reducer = NoiseReducer::new(1.0);
        let out = reducer.process(&vec![0.0; FRAME * 3]);
        assert_eq!(out.len() % HOP, 0);
        assert!(out.len() >= FRAME * 3 - FRAME);
    }
}
