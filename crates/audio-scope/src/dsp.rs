use std::f64::consts::TAU;
use std::ops::Range;
use std::sync::Arc;

use rustfft::{num_complex::Complex, Fft, FftPlanner};

const MIN_FFT: usize = 64;
pub const MAX_FFT: usize = 2048;
const FFT_SIZES: usize = 6;
const SPECTRUM_BARS: usize = 64;

struct Window {
    weights: Vec<f64>,
    sum: f64,
}

impl Window {
    fn new(len: usize) -> Self {
        let weights: Vec<f64> = (0..len)
            .map(|index| {
                if len > 1 {
                    0.5 - 0.5 * (TAU * index as f64 / (len - 1) as f64).cos()
                } else {
                    1.0
                }
            })
            .collect();
        let sum = weights.iter().sum();
        Self { weights, sum }
    }
}

struct Transform {
    forward: Arc<dyn Fft<f64>>,
    inverse: Arc<dyn Fft<f64>>,
    // Positive-frequency bins; negative-frequency partners are copied on demand.
    bands: [[Range<usize>; 3]; 3],
    bars: [Range<usize>; SPECTRUM_BARS],
    nearest_bins: [usize; SPECTRUM_BARS],
}

impl Transform {
    fn new(planner: &mut FftPlanner<f64>, n: usize, sample_rate: f64) -> Self {
        let nyquist = sample_rate / 2.0;
        let bin =
            |frequency: f64| ((frequency * n as f64 / sample_rate).ceil() as usize).min(n / 2 + 1);
        let ranges = |edges: &[(f64, f64)]| {
            std::array::from_fn(|index| match edges.get(index) {
                Some(&(low, high)) => {
                    let start = bin(low);
                    start..bin(high).max(start)
                }
                None => 0..0,
            })
        };
        let bands = [
            ranges(&[(20.0, nyquist)]),
            ranges(&[(20.0, 800.0), (800.0, nyquist)]),
            ranges(&[(20.0, 250.0), (250.0, 2500.0), (2500.0, nyquist)]),
        ];
        let bars = std::array::from_fn(|index| {
            if nyquist <= 20.0 {
                return 0..0;
            }
            let ratio = nyquist / 20.0;
            let low = 20.0 * ratio.powf(index as f64 / SPECTRUM_BARS as f64);
            let high = 20.0 * ratio.powf((index + 1) as f64 / SPECTRUM_BARS as f64);
            let start = bin(low);
            let end = if index + 1 == SPECTRUM_BARS {
                n / 2 + 1
            } else {
                bin(high)
            };
            start..end.max(start)
        });
        // Log bands can be narrower than one FFT bin. Sample the nearest
        // positive-frequency bin for those bands instead of leaving display gaps.
        let nearest_bins = std::array::from_fn(|index| {
            if nyquist <= 20.0 {
                return 1;
            }
            let ratio = nyquist / 20.0;
            let low = 20.0 * ratio.powf(index as f64 / SPECTRUM_BARS as f64);
            let high = 20.0 * ratio.powf((index + 1) as f64 / SPECTRUM_BARS as f64);
            let center = (low * high).sqrt();
            ((center * n as f64 / sample_rate).round() as usize).clamp(1, n / 2)
        });
        Self {
            forward: planner.plan_fft_forward(n),
            inverse: planner.plan_fft_inverse(n),
            bands,
            bars,
            nearest_bins,
        }
    }
}

/// Cached analysis of the most recent audio samples.
///
/// Two/three-wave levels are relative RMS shares; without wave synthesis they
/// are relative spectral-energy shares. One-wave mode preserves the raw input
/// and its absolute RMS. Spectrum bars map -80..0 dBFS to 0..1.
/// Plans and bin ranges are prepared once. Each usable Hann-window length is
/// cached on first encounter; work buffers and band outputs retain capacity.
/// In spectrum-only frames, no inverse transform is executed.
pub struct Dsp {
    pub waves: [Vec<f64>; 3],
    pub levels: [f64; 3],
    pub spectrum: [f64; SPECTRUM_BARS],
    transforms: [Transform; FFT_SIZES],
    windows: Vec<Option<Window>>,
    fft: Vec<Complex<f64>>,
    band: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
}

impl Dsp {
    pub fn new(sample_rate: u32) -> Self {
        let mut planner = FftPlanner::<f64>::new();
        let transforms: [Transform; FFT_SIZES] = std::array::from_fn(|index| {
            Transform::new(&mut planner, MIN_FFT << index, sample_rate.max(1) as f64)
        });
        let scratch_len = transforms
            .iter()
            .map(|transform| {
                transform
                    .forward
                    .get_inplace_scratch_len()
                    .max(transform.inverse.get_inplace_scratch_len())
            })
            .max()
            .unwrap_or(0);
        Self {
            waves: std::array::from_fn(|_| Vec::with_capacity(MAX_FFT)),
            levels: [0.0; 3],
            spectrum: [0.0; SPECTRUM_BARS],
            transforms,
            windows: (0..=MAX_FFT).map(|_| None).collect(),
            fft: vec![Complex::new(0.0, 0.0); MAX_FFT],
            band: vec![Complex::new(0.0, 0.0); MAX_FFT],
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
        }
    }

    pub fn analyze(&mut self, samples: &[f64], wave_count: usize, make_waves: bool) {
        self.levels.fill(0.0);
        self.spectrum.fill(0.0);
        for wave in &mut self.waves {
            wave.clear();
        }
        if samples.is_empty() {
            return;
        }

        let wave_count = wave_count.clamp(1, 3);
        if wave_count == 1 {
            self.levels[0] = (samples.iter().map(|sample| sample * sample).sum::<f64>()
                / samples.len() as f64)
                .sqrt();
            if make_waves {
                self.waves[0].extend_from_slice(samples);
                return;
            }
        }
        // Cap before rounding to avoid overflow even for extremely long input.
        let n = samples.len().clamp(MIN_FFT, MAX_FFT).next_power_of_two();
        let usable = samples.len().min(n);
        let recent = &samples[samples.len() - usable..];
        let window = self.windows[usable].get_or_insert_with(|| Window::new(usable));
        let transform = &self.transforms[(n.trailing_zeros() - MIN_FFT.trailing_zeros()) as usize];
        self.fft[..n].fill(Complex::new(0.0, 0.0));
        for ((value, &sample), &weight) in self.fft[..usable]
            .iter_mut()
            .zip(recent)
            .zip(&window.weights)
        {
            value.re = sample * weight;
        }
        transform
            .forward
            .process_with_scratch(&mut self.fft[..n], &mut self.scratch);

        // Coherent Hann gain makes a full-scale bin-centered sine approximately
        // 0 dBFS regardless of FFT/window length. DC and Nyquist have no mirror.
        let gain = window.sum.max(f64::MIN_POSITIVE);
        for (index, (bar, range)) in self.spectrum.iter_mut().zip(&transform.bars).enumerate() {
            let mut amplitude: f64 = 0.0;
            let bins = if range.is_empty() {
                let nearest = transform.nearest_bins[index];
                nearest..nearest + 1
            } else {
                range.clone()
            };
            for bin in bins {
                let factor = if bin == 0 || bin == n / 2 { 1.0 } else { 2.0 };
                amplitude = amplitude.max(self.fft[bin].norm_sqr() * factor * factor);
            }
            if amplitude > 0.0 {
                let db = 20.0 * (amplitude.sqrt() / gain).log10();
                *bar = ((db + 80.0) / 80.0).clamp(0.0, 1.0);
            }
        }

        if wave_count == 1 {
            return;
        }

        for (band_index, range) in transform.bands[wave_count - 1]
            .iter()
            .take(wave_count)
            .enumerate()
        {
            if make_waves {
                self.band[..n].fill(Complex::new(0.0, 0.0));
                for bin in range.clone() {
                    self.band[bin] = self.fft[bin];
                    if bin != 0 && bin != n / 2 {
                        self.band[n - bin] = self.fft[n - bin];
                    }
                }
                transform
                    .inverse
                    .process_with_scratch(&mut self.band[..n], &mut self.scratch);
                let wave = &mut self.waves[band_index];
                wave.extend(
                    self.band[..usable]
                        .iter()
                        .map(|sample| (sample.re / n as f64).clamp(-1.0, 1.0)),
                );
                self.levels[band_index] =
                    (wave.iter().map(|sample| sample * sample).sum::<f64>() / usable as f64).sqrt();
            } else {
                self.levels[band_index] = range
                    .clone()
                    .map(|bin| {
                        let mirrors = if bin == 0 || bin == n / 2 { 1.0 } else { 2.0 };
                        self.fft[bin].norm_sqr() * mirrors
                    })
                    .sum();
            }
        }
        let total = self.levels.iter().sum::<f64>();
        if total > 0.0 {
            for level in &mut self.levels {
                *level = (*level / total).clamp(0.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Dsp, MAX_FFT, SPECTRUM_BARS};
    use std::f64::consts::TAU;

    const RATE: u32 = 48_000;

    fn sine(bin: usize, len: usize, amplitude: f64) -> Vec<f64> {
        (0..len)
            .map(|index| amplitude * (TAU * bin as f64 * index as f64 / MAX_FFT as f64).sin())
            .collect()
    }

    fn peak_bar(spectrum: &[f64; SPECTRUM_BARS]) -> usize {
        spectrum
            .iter()
            .enumerate()
            .fold(
                (0, 0.0),
                |peak, (index, &value)| {
                    if value > peak.1 {
                        (index, value)
                    } else {
                        peak
                    }
                },
            )
            .0
    }

    #[test]
    fn sine_peaks_follow_logarithmic_frequency_bars() {
        let mut dsp = Dsp::new(RATE);
        for bin in [8, 43, 256, 900] {
            dsp.analyze(&sine(bin, MAX_FFT, 0.5), 3, false);
            let frequency = bin as f64 * RATE as f64 / MAX_FFT as f64;
            let expected = ((frequency / 20.0).ln() / (RATE as f64 / 40.0).ln()
                * SPECTRUM_BARS as f64)
                .floor() as usize;
            assert_eq!(peak_bar(&dsp.spectrum), expected);
            assert!((dsp.spectrum[expected] - 0.9247425).abs() < 0.002);
            assert!(dsp.spectrum.iter().all(|value| (0.0..=1.0).contains(value)));
        }
    }

    #[test]
    fn short_fft_keeps_unresolved_low_log_bands_reactive() {
        let mut dsp = Dsp::new(RATE);
        let len = 256;
        let samples: Vec<_> = (0..len)
            .map(|index| 0.5 * (TAU * index as f64 / len as f64).sin())
            .collect();
        dsp.analyze(&samples, 3, false);

        assert!(dsp.spectrum[..=20].iter().all(|&level| level > 0.9));
        assert_eq!(dsp.spectrum[40], 0.0);
    }

    #[test]
    fn spectrum_db_tracks_amplitude() {
        let mut dsp = Dsp::new(RATE);
        dsp.analyze(&sine(43, MAX_FFT, 1.0), 3, false);
        let index = peak_bar(&dsp.spectrum);
        let full_scale = dsp.spectrum[index];
        dsp.analyze(&sine(43, MAX_FFT, 0.5), 3, false);
        assert!((full_scale - dsp.spectrum[index] - 6.0205999 / 80.0).abs() < 0.0001);
    }

    #[test]
    fn three_wave_bands_isolate_bass_mid_and_treble() {
        let mut dsp = Dsp::new(RATE);
        for (band, bin) in [5, 43, 256].into_iter().enumerate() {
            dsp.analyze(&sine(bin, MAX_FFT, 0.8), 3, true);
            assert!(dsp.levels[band] > 0.99, "levels: {:?}", dsp.levels);
            for (index, wave) in dsp.waves.iter().enumerate() {
                assert_eq!(wave.len(), MAX_FFT);
                let rms = (wave.iter().map(|sample| sample * sample).sum::<f64>()
                    / wave.len() as f64)
                    .sqrt();
                if index == band {
                    assert!(rms > 0.3);
                } else {
                    assert!(rms < 0.003);
                }
            }
        }
    }

    #[test]
    fn two_wave_bands_split_at_eight_hundred_hertz() {
        let mut dsp = Dsp::new(RATE);
        for (band, bin) in [13, 85].into_iter().enumerate() {
            dsp.analyze(&sine(bin, MAX_FFT, 0.8), 2, true);
            assert!(dsp.levels[band] > 0.99);
            assert!(dsp.waves[2].is_empty());
            assert_eq!(dsp.levels[2], 0.0);
        }
    }

    #[test]
    fn mixed_bands_use_relative_rms_or_spectral_energy() {
        let mut dsp = Dsp::new(RATE);
        let mut samples = sine(5, MAX_FFT, 0.8);
        for (bin, amplitude) in [(43, 0.4), (256, 0.2)] {
            for (sample, extra) in samples.iter_mut().zip(sine(bin, MAX_FFT, amplitude)) {
                *sample += extra;
            }
        }
        dsp.analyze(&samples, 3, true);
        for (&actual, expected) in dsp.levels.iter().zip([4.0 / 7.0, 2.0 / 7.0, 1.0 / 7.0]) {
            assert!((actual - expected).abs() < 0.005);
        }
        dsp.analyze(&samples, 3, false);
        for (&actual, expected) in dsp.levels.iter().zip([16.0 / 21.0, 4.0 / 21.0, 1.0 / 21.0]) {
            assert!((actual - expected).abs() < 0.005);
        }
    }

    #[test]
    fn spectrum_only_clears_waves_and_reports_band_energy() {
        let mut dsp = Dsp::new(RATE);
        let samples = sine(256, MAX_FFT, 0.5);
        dsp.analyze(&samples, 3, true);
        let spectrum = dsp.spectrum;
        dsp.analyze(&samples, 3, false);
        assert!(dsp.waves.iter().all(Vec::is_empty));
        assert_eq!(dsp.spectrum, spectrum);
        assert!(dsp.levels[2] > 0.999);
        assert!((dsp.levels.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn one_wave_retains_full_raw_input_and_absolute_rms() {
        let mut dsp = Dsp::new(RATE);
        let samples = sine(43, MAX_FFT * 2, 0.5);
        dsp.analyze(&samples, 1, true);
        assert_eq!(dsp.waves[0], samples);
        assert!(dsp.waves[1].is_empty());
        assert!(dsp.waves[2].is_empty());
        assert!((dsp.levels[0] - 0.5 / 2.0_f64.sqrt()).abs() < 1e-12);
        assert_eq!(&dsp.levels[1..], &[0.0, 0.0]);
    }

    #[test]
    fn size_transitions_and_empty_input_clear_old_output() {
        let mut dsp = Dsp::new(RATE);
        for len in [100, MAX_FFT, 71, MAX_FFT * 2, 1, 2, 100] {
            dsp.analyze(&sine(43, len, 0.5), 3, true);
            assert!(dsp.waves.iter().all(|wave| wave.len() == len.min(MAX_FFT)));
            assert!(dsp.levels.iter().all(|value| value.is_finite()));
            assert!(dsp.spectrum.iter().all(|value| value.is_finite()));
        }
        dsp.analyze(&[], 3, true);
        assert!(dsp.waves.iter().all(Vec::is_empty));
        assert_eq!(dsp.levels, [0.0; 3]);
        assert_eq!(dsp.spectrum, [0.0; SPECTRUM_BARS]);
        dsp.analyze(&[0.0; 128], 3, false);
        assert_eq!(dsp.levels, [0.0; 3]);
        assert_eq!(dsp.spectrum, [0.0; SPECTRUM_BARS]);
    }

    #[test]
    fn fft_uses_only_the_latest_capped_samples() {
        let mut dsp = Dsp::new(RATE);
        let recent = sine(43, MAX_FFT, 0.5);
        let mut samples = sine(256, MAX_FFT, 0.8);
        samples.extend_from_slice(&recent);
        dsp.analyze(&samples, 3, true);
        let waves = dsp.waves.clone();
        let spectrum = dsp.spectrum;
        let levels = dsp.levels;
        dsp.analyze(&recent, 3, true);
        assert_eq!(dsp.waves, waves);
        assert_eq!(dsp.spectrum, spectrum);
        assert_eq!(dsp.levels, levels);
    }
}
