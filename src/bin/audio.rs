#[cfg(feature = "audio")]
use clap::{Parser, ValueEnum};
#[cfg(feature = "audio")]
use ratatui::style::Color;
#[cfg(feature = "audio")]
use rustfft::{num_complex::Complex, FftPlanner};
#[cfg(feature = "audio")]
use scope_studio::{
    data::audio::AudioSource, render::Series, run_app, AppConfig, Matrix, Renderer, VisualStyle,
};

#[cfg(feature = "audio")]
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Theme {
    /// original color defaults used by the initial 3-band visualizer build
    Original,
    /// Bright defaults (red / yellow / magenta)
    Classic,
    /// Cyan / magenta / yellow
    Neon,
    /// Green / aqua / blue
    Ocean,
    /// Soft monochrome grayscale
    Mono,
    /// Muted seafoam and rose tones from pinboard palette
    Doki,
}

#[cfg(feature = "audio")]
impl Theme {
    fn colors(&self) -> [Color; 3] {
        match self {
            Theme::Original => [Color::Cyan, Color::Green, Color::Blue],
            Theme::Classic => [Color::Red, Color::Yellow, Color::Magenta],
            Theme::Neon => [Color::Cyan, Color::Magenta, Color::Yellow],
            Theme::Ocean => [Color::Blue, Color::Cyan, Color::Green],
            Theme::Mono => [Color::Gray, Color::DarkGray, Color::White],
            Theme::Doki => [
                Color::Rgb(120, 160, 168),
                Color::Rgb(152, 120, 152),
                Color::Rgb(136, 208, 192),
            ],
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Theme::Original => "original",
            Theme::Classic => "classic",
            Theme::Neon => "neon",
            Theme::Ocean => "ocean",
            Theme::Mono => "mono",
            Theme::Doki => "doki",
        }
    }
}

#[cfg(feature = "audio")]
const BACKGROUND_PRESETS: [(Color, &str); 8] = [
    (Color::Reset, "terminal"),
    (Color::Black, "black"),
    (Color::Rgb(10, 12, 14), "classic"),
    (Color::Rgb(6, 0, 18), "neon"),
    (Color::Rgb(3, 10, 24), "ocean"),
    (Color::Rgb(16, 16, 16), "mono"),
    (Color::Rgb(0, 8, 24), "indigo"),
    (Color::Rgb(30, 32, 34), "doki"),
];

#[cfg(feature = "audio")]
fn theme_background_idx(theme: Theme) -> usize {
    match theme {
        Theme::Original => 0,
        Theme::Classic => 2,
        Theme::Neon => 3,
        Theme::Ocean => 4,
        Theme::Mono => 5,
        Theme::Doki => 7,
    }
}

#[cfg(feature = "audio")]
#[derive(Parser)]
#[command(name = "doki")]
struct Cli {
    /// Input sample rate
    #[arg(short = 'r', long, default_value_t = 44_100)]
    sample_rate: u32,
    /// Input channels (1-2)
    #[arg(short = 'c', long, default_value_t = 1)]
    channels: usize,
    /// Input device name (exact match from available input devices)
    ///
    /// Pass `auto` (default) to auto-select input capture.
    ///
    /// On Linux, this prefers system/monitor sources; on other platforms, safe input capture is used.
    #[arg(short = 'd', long, default_value = "auto")]
    device: String,
    /// Prefer safe non-monitor input sources instead of system audio capture
    #[arg(long, default_value_t = false)]
    safe: bool,
    /// Visualization theme
    #[arg(short = 't', long, value_enum, default_value_t = Theme::Original)]
    theme: Theme,
    /// Print input device list and exit
    #[arg(short = 'l', long)]
    list_devices: bool,
    /// Ring history
    #[arg(short = 'n', long, default_value_t = 256)]
    history: usize,
    #[arg(short = 'w', long, default_value_t = 512)]
    width: usize,
    /// Polling sleep in ms
    #[arg(short = 's', long, default_value_t = 16)]
    sleep_ms: u64,
}

#[cfg(feature = "audio")]
struct AudioRenderer {
    sample_rate: u32,
    theme: Theme,
    background_idx: usize,
    wave_count: usize,
    peak: f64,
    rms: f64,
    clip_pct: f64,
    bass: f64,
    mid: f64,
    treble: f64,
}

#[cfg(feature = "audio")]
impl AudioRenderer {
    fn new(sample_rate: u32, theme: Theme) -> Self {
        Self {
            sample_rate,
            theme,
            background_idx: theme_background_idx(theme),
            wave_count: 3,
            peak: 0.0,
            rms: 0.0,
            clip_pct: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
        }
    }

    fn visual_style_name(&self) -> &'static str {
        "wave"
    }

    fn background_name(&self) -> &'static str {
        BACKGROUND_PRESETS[self.background_idx % BACKGROUND_PRESETS.len()].1
    }

    fn cycle_wave_count(&mut self) {
        self.wave_count = match self.wave_count {
            3 => 2,
            2 => 1,
            _ => 3,
        };
    }

    fn trim_samples(row: &[f64], cfg: &AppConfig) -> Vec<f64> {
        let start = row.len().saturating_sub(cfg.width);
        row.iter().skip(start).copied().collect()
    }

    fn update_stats(&mut self, samples: &[f64]) {
        if samples.is_empty() {
            self.peak = 0.0;
            self.rms = 0.0;
            self.clip_pct = 0.0;
            self.bass = 0.0;
            self.mid = 0.0;
            self.treble = 0.0;
            return;
        }

        let mut sum_sq = 0.0;
        let mut peak: f64 = 0.0;
        let mut clipped = 0usize;
        for &sample in samples {
            let abs = sample.abs();
            peak = peak.max(abs);
            sum_sq += sample * sample;
            if abs >= 0.98 {
                clipped += 1;
            }
        }

        self.peak = peak;
        self.rms = (sum_sq / samples.len() as f64).sqrt();
        self.clip_pct = (clipped as f64 * 100.0) / samples.len() as f64;
    }

    fn band_waveforms(&self, cfg: &AppConfig, samples: &[f64]) -> (Vec<Vec<f64>>, [f64; 3]) {
        if samples.is_empty() {
            let width = cfg.width.max(1);
            let count = self.wave_count.clamp(1, 3);
            return (vec![vec![0.0; width]; count], [0.0; 3]);
        }

        if self.wave_count == 1 {
            return (vec![samples.to_vec()], [self.rms, 0.0, 0.0]);
        }

        let n = samples.len().max(64).next_power_of_two().min(2048);
        let usable = samples.len().min(n);
        let offset = samples.len().saturating_sub(usable);

        let mut planner = FftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(n);
        let mut spectrum = vec![Complex::new(0.0, 0.0); n];

        for (i, &sample) in samples[offset..].iter().enumerate() {
            let hann = if usable > 1 {
                let phase = (2.0 * std::f64::consts::PI * i as f64) / (usable as f64 - 1.0);
                0.5 - 0.5 * phase.cos()
            } else {
                1.0
            };
            spectrum[i].re = sample * hann;
        }

        fft.process(&mut spectrum);

        let sample_rate = self.sample_rate.max(8_000) as f64;
        let bands: &[(f64, f64)] = if self.wave_count == 2 {
            &[(20.0, 800.0), (800.0, sample_rate / 2.0)]
        } else {
            &[
                (20.0, 250.0),
                (250.0, 2_500.0),
                (2_500.0, sample_rate / 2.0),
            ]
        };

        let mut waveforms = Vec::with_capacity(3);
        let mut levels = [0.0; 3];

        for (band_idx, &(low, high)) in bands.iter().enumerate() {
            let mut band_spectrum = vec![Complex::new(0.0, 0.0); n];
            for bin in 0..=n / 2 {
                let freq = (bin as f64 * sample_rate) / n as f64;
                if freq >= low && freq < high {
                    band_spectrum[bin] = spectrum[bin];
                    if bin != 0 && bin != n / 2 {
                        band_spectrum[n - bin] = spectrum[n - bin];
                    }
                }
            }

            let ifft = planner.plan_fft_inverse(n);
            ifft.process(&mut band_spectrum);
            let waveform: Vec<f64> = band_spectrum
                .into_iter()
                .take(usable)
                .map(|value| (value.re / n as f64).clamp(-1.0, 1.0))
                .collect();

            let sum_sq = waveform.iter().map(|sample| sample * sample).sum::<f64>();
            if !waveform.is_empty() {
                levels[band_idx] = (sum_sq / waveform.len() as f64).sqrt();
            }

            waveforms.push(waveform);
        }

        let total = levels.iter().sum::<f64>();
        if total > 0.0 {
            for level in &mut levels {
                *level = (*level / total).clamp(0.0, 1.0);
            }
        }

        (waveforms, levels)
    }
}

#[cfg(feature = "audio")]
impl Renderer for AudioRenderer {
    fn mode_name(&self) -> &'static str {
        "audio"
    }

    fn header(&self) -> String {
        format!(
            "theme {:<7} | bg {:<8} | style {:<6} | waves {} |  bass {:.2}  mid {:.2}  treble {:.2}  |  peak {:.2}  rms {:.2}  clip {:>3.0}%",
            self.theme.title(),
            self.background_name(),
            self.visual_style_name(),
            self.wave_count,
            self.bass,
            self.mid,
            self.treble,
            self.peak,
            self.rms,
            self.clip_pct
        )
    }

    fn y_bounds(&self, _cfg: &AppConfig) -> (f64, f64) {
        (-1.0, 1.0)
    }

    fn visual_style(&self) -> VisualStyle {
        VisualStyle::Line
    }

    fn background_color(&self) -> Color {
        BACKGROUND_PRESETS[self.background_idx % BACKGROUND_PRESETS.len()].0
    }

    fn process(&mut self, cfg: &AppConfig, frame: &Matrix) -> Vec<Series> {
        let samples = match frame.first() {
            Some(row) => Self::trim_samples(row, cfg),
            None => Vec::new(),
        };
        self.update_stats(&samples);

        let (band_waveforms, levels) = self.band_waveforms(cfg, &samples);
        self.bass = levels[0];
        self.mid = levels[1];
        self.treble = levels[2];

        let band_names: &[&str] = match self.wave_count {
            1 => &["wave"],
            2 => &["low", "high"],
            _ => &["bass", "mid", "treble"],
        };
        let band_colors = self.theme.colors();
        let mut series = Vec::with_capacity(band_names.len());
        for ((name, color), waveform) in band_names
            .iter()
            .zip(band_colors.iter())
            .zip(band_waveforms)
        {
            series.push(Series {
                name: (*name).to_string(),
                color: *color,
                samples: waveform,
            });
        }

        series
    }
    fn handle_event(&mut self, event: &crossterm::event::Event, _cfg: &mut AppConfig) {
        if let crossterm::event::Event::Key(key) = event {
            if key.kind == crossterm::event::KeyEventKind::Press {
                match key.code {
                    crossterm::event::KeyCode::Char('t') => {
                        self.theme = match self.theme {
                            Theme::Original => Theme::Classic,
                            Theme::Classic => Theme::Neon,
                            Theme::Neon => Theme::Ocean,
                            Theme::Ocean => Theme::Mono,
                            Theme::Mono => Theme::Doki,
                            Theme::Doki => Theme::Original,
                        };
                    }
                    crossterm::event::KeyCode::Char('b') => {
                        self.background_idx = (self.background_idx + 1) % BACKGROUND_PRESETS.len();
                    }
                    crossterm::event::KeyCode::Char('w') => {
                        self.cycle_wave_count();
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(feature = "audio")]
fn main() {
    let args = Cli::parse();

    if args.list_devices {
        let list_system_audio = !args.safe && AudioSource::supports_system_audio_capture();
        if let Err(err) = AudioSource::list_input_devices(list_system_audio) {
            eprintln!("could not list devices: {err}");
        }
        return;
    }

    let use_system_audio = !args.safe && AudioSource::supports_system_audio_capture();
    if use_system_audio {
        eprintln!(
            "system-audio mode active: this will capture playback/monitor sources by default"
        );
    } else if !AudioSource::supports_system_audio_capture() {
        eprintln!("system-audio mode is not available on this platform; using safe input capture");
    }

    let config = AppConfig {
        width: args.width,
        sleep_ms: args.sleep_ms,
        ..Default::default()
    };

    let source = match AudioSource::new(
        args.device,
        args.sample_rate,
        args.channels,
        args.history,
        use_system_audio,
    ) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("audio source init failed: {err}");
            return;
        }
    };

    if let Err(err) = run_app(
        source,
        AudioRenderer::new(args.sample_rate, args.theme),
        config,
    ) {
        eprintln!("audio-scope failed: {err:?}");
    }
}

#[cfg(not(feature = "audio"))]
fn main() {
    eprintln!(
        "audio-scope requires the `audio` feature: cargo run --bin audio-scope --features audio"
    );
}
