#[cfg(feature = "audio")]
use std::collections::VecDeque;

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
    /// Aquamarine-inspired theme from reference image
    Pinboard,
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
            Theme::Pinboard => [
                Color::Rgb(67, 102, 112),
                Color::Rgb(92, 70, 116),
                Color::Rgb(73, 82, 110),
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
            Theme::Pinboard => "pinboard",
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
        Theme::Pinboard => 7,
    }
}

#[cfg(feature = "audio")]
fn shift_hue(base: Color, hue_shift: f64) -> Color {
    fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
        let r = r as f64 / 255.0;
        let g = g as f64 / 255.0;
        let b = b as f64 / 255.0;

        let max = r.max(g.max(b));
        let min = r.min(g.min(b));
        let delta = max - min;

        let hue = if delta == 0.0 {
            0.0
        } else if max == r {
            (g - b) / delta % 6.0 * 60.0
        } else if max == g {
            ((b - r) / delta + 2.0) * 60.0
        } else {
            ((r - g) / delta + 4.0) * 60.0
        };

        let sat = if max == 0.0 { 0.0 } else { delta / max };
        (hue.rem_euclid(360.0), sat, max)
    }

    fn hsv_to_rgb(hue: f64, sat: f64, val: f64) -> (u8, u8, u8) {
        let c = val * sat;
        let h = hue.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - ((h % 2.0) - 1.0).abs());

        let (r1, g1, b1) = match (h.floor() as u8) % 6 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };

        let m = val - c;
        let to_u8 = |v: f64| ((v + m) * 255.0).clamp(0.0, 255.0).round() as u8;
        (to_u8(r1), to_u8(g1), to_u8(b1))
    }

    match base {
        Color::Rgb(r, g, b) => {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let (r, g, b) = hsv_to_rgb(h + hue_shift, s, v);
            Color::Rgb(r, g, b)
        }
        _ => base,
    }
}

#[cfg(feature = "audio")]
#[derive(Clone, Copy, Debug, ValueEnum)]
enum VisualMode {
    /// Standard time-domain waveform with overlaid 3/2/1-band traces
    Line,
    /// Multi-frame persistence trail visualization
    Persistence,
}

#[cfg(feature = "audio")]
impl From<VisualMode> for VisualStyle {
    fn from(mode: VisualMode) -> Self {
        match mode {
            VisualMode::Line => VisualStyle::Line,
            VisualMode::Persistence => VisualStyle::Persistence,
        }
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
    /// Enable slow RGB hue-cycling on waves
    #[arg(long, default_value_t = false)]
    rgb_wave: bool,
    /// RGB hue increment per rendered frame (degrees). Larger values = faster color cycling.
    #[arg(long, default_value_t = 0.5)]
    rgb_speed: f64,
    /// Initial visual mode
    #[arg(short = 'm', long, value_enum, default_value_t = VisualMode::Line)]
    visual_mode: VisualMode,
    /// History depth used for persistence mode
    #[arg(long, default_value_t = 12)]
    persistence_window: usize,
    /// Ring history (lower values reduce waveform + FFT work)
    #[arg(short = 'n', long, default_value_t = 256)]
    history: usize,
    /// Number of waveform bands (1 disables band-splitting FFT work)
    #[arg(long, default_value_t = 3)]
    wave_count: usize,
    /// Render width in columns
    #[arg(short = 'w', long, default_value_t = 512)]
    width: usize,
    /// Polling sleep in ms (24 FPS target)
    #[arg(short = 's', long, default_value_t = 42)]
    sleep_ms: u64,
    /// Print input device list and exit
    #[arg(short = 'l', long)]
    list_devices: bool,
}

#[cfg(feature = "audio")]
struct AudioRenderer {
    sample_rate: u32,
    theme: Theme,
    background_idx: usize,
    visual_style: VisualStyle,
    persistence_window: usize,
    persistence_history: VecDeque<Vec<Vec<f64>>>,
    wave_count: usize,
    rgb_phase: f64,
    rgb_wave: bool,
    rgb_speed: f64,
    peak: f64,
    rms: f64,
    clip_pct: f64,
    bass: f64,
    mid: f64,
    treble: f64,
    fft_planner: FftPlanner<f64>,
    hann: Vec<f64>,
    spectrum: Vec<Complex<f64>>,
    band_spectrum: Vec<Complex<f64>>,
    bin_freqs: Vec<f64>,
}

#[cfg(feature = "audio")]
impl AudioRenderer {
    fn new(
        sample_rate: u32,
        theme: Theme,
        visual_style: VisualStyle,
        persistence_window: usize,
        rgb_wave: bool,
        rgb_speed: f64,
        wave_count: usize,
    ) -> Self {
        Self {
            sample_rate,
            theme,
            background_idx: theme_background_idx(theme),
            visual_style,
            persistence_window: persistence_window.max(1),
            persistence_history: VecDeque::new(),
            wave_count: wave_count.clamp(1, 3),
            rgb_phase: 0.0,
            rgb_wave,
            rgb_speed: if rgb_speed.is_finite() {
                rgb_speed.abs()
            } else {
                0.0
            },
            peak: 0.0,
            rms: 0.0,
            clip_pct: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            fft_planner: FftPlanner::<f64>::new(),
            hann: Vec::new(),
            spectrum: Vec::new(),
            band_spectrum: Vec::new(),
            bin_freqs: Vec::new(),
        }
    }

    fn visual_style_name(&self) -> &'static str {
        match self.visual_style {
            VisualStyle::Line => "line",
            VisualStyle::Persistence => "persistence",
        }
    }

    fn background_name(&self) -> &'static str {
        BACKGROUND_PRESETS[self.background_idx % BACKGROUND_PRESETS.len()].1
    }

    fn cycle_visual_style(&mut self) {
        self.visual_style = match self.visual_style {
            VisualStyle::Line => VisualStyle::Persistence,
            VisualStyle::Persistence => VisualStyle::Line,
        }
    }

    fn cycle_wave_count(&mut self) {
        self.wave_count = match self.wave_count {
            3 => 2,
            2 => 1,
            _ => 3,
        };
    }

    fn trim_samples<'a>(row: &'a [f64], cfg: &AppConfig) -> &'a [f64] {
        let start = row.len().saturating_sub(cfg.width);
        &row[start..]
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

    fn band_waveforms(&mut self, cfg: &AppConfig, samples: &[f64]) -> (Vec<Vec<f64>>, [f64; 3]) {
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

        let fft = self.fft_planner.plan_fft_forward(n);
        let ifft = self.fft_planner.plan_fft_inverse(n);
        let sample_rate = self.sample_rate.max(8_000) as f64;
        let zero = Complex::new(0.0, 0.0);

        if self.hann.len() != usable {
            self.hann.resize(usable, 0.0);
            for (i, value) in self.hann.iter_mut().enumerate() {
                *value = if usable > 1 {
                    let phase = (2.0 * std::f64::consts::PI * i as f64) / (usable as f64 - 1.0);
                    0.5 - 0.5 * phase.cos()
                } else {
                    1.0
                };
            }
        }

        if self.spectrum.len() != n {
            self.spectrum.resize(n, zero);
        }
        if self.band_spectrum.len() != n {
            self.band_spectrum.resize(n, zero);
        }
        let bin_count = n / 2 + 1;
        if self.bin_freqs.len() != bin_count {
            self.bin_freqs.resize(bin_count, 0.0);
            for (bin, freq) in self.bin_freqs.iter_mut().enumerate() {
                *freq = (bin as f64 * sample_rate) / n as f64;
            }
        }

        // Single forward FFT of the windowed signal.
        self.spectrum.fill(zero);
        for (i, &sample) in samples[offset..].iter().enumerate() {
            self.spectrum[i].re = sample * self.hann[i];
        }
        fft.process(&mut self.spectrum);

        let bands: &[(f64, f64)] = if self.wave_count == 2 {
            &[(20.0, 800.0), (800.0, sample_rate / 2.0)]
        } else {
            &[
                (20.0, 250.0),
                (250.0, 2_500.0),
                (2_500.0, sample_rate / 2.0),
            ]
        };

        let mut waveforms = Vec::with_capacity(bands.len());
        let mut levels = [0.0; 3];

        for (band_idx, &(low, high)) in bands.iter().enumerate() {
            self.band_spectrum.copy_from_slice(&self.spectrum);
            for bin in 0..=n / 2 {
                let freq = self.bin_freqs[bin];
                if freq < low || freq >= high {
                    self.band_spectrum[bin] = zero;
                    if bin != 0 && bin != n / 2 {
                        self.band_spectrum[n - bin] = zero;
                    }
                }
            }

            ifft.process(&mut self.band_spectrum);
            let waveform: Vec<f64> = self.band_spectrum[..usable]
                .iter()
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

fn fade_color(base: Color, intensity: f64) -> Color {
    let intensity = intensity.clamp(0.2, 1.0);
    match base {
        Color::Rgb(r, g, b) => Color::Rgb(
            ((r as f64) * intensity).clamp(0.0, 255.0).round() as u8,
            ((g as f64) * intensity).clamp(0.0, 255.0).round() as u8,
            ((b as f64) * intensity).clamp(0.0, 255.0).round() as u8,
        ),
        other => other,
    }
}

#[cfg(feature = "audio")]
impl Renderer for AudioRenderer {
    fn header(&self) -> String {
        format!(
            "theme {:<9} | bg {:<10} | style {:<10} | waves {} | rgb {:<3} | bass {:.2}  mid {:.2}  treble {:.2}  |  peak {:.2}  rms {:.2}  clip {:>3.0}%",
            self.theme.title(),
            self.background_name(),
            self.visual_style_name(),
            self.wave_count,
            if self.rgb_wave { "on" } else { "off" },
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
        self.visual_style
    }

    fn background_color(&self) -> Color {
        BACKGROUND_PRESETS[self.background_idx % BACKGROUND_PRESETS.len()].0
    }

    fn process(&mut self, cfg: &AppConfig, frame: &Matrix) -> Vec<Series> {
        let style = self.visual_style;
        let samples: &[f64] = match frame.first() {
            Some(row) => Self::trim_samples(row, cfg),
            None => &[],
        };
        self.update_stats(samples);

        let (band_waveforms, levels) = self.band_waveforms(cfg, samples);
        self.bass = levels[0];
        self.mid = levels[1];
        self.treble = levels[2];

        let band_names: &[&str] = match self.wave_count {
            1 => &["wave"],
            2 => &["low", "high"],
            _ => &["bass", "mid", "treble"],
        };
        if self.rgb_wave {
            self.rgb_phase = (self.rgb_phase + self.rgb_speed) % 360.0;
        }
        let mut band_colors = self.theme.colors();
        if self.rgb_wave {
            for (idx, color) in band_colors.iter_mut().enumerate() {
                *color = shift_hue(*color, self.rgb_phase + (idx as f64) * 45.0);
            }
        }

        let mut series = Vec::new();
        match style {
            VisualStyle::Persistence => {
                if let Some(front) = self.persistence_history.front() {
                    if front.len() != band_names.len() {
                        self.persistence_history.clear();
                    }
                }

                self.persistence_history.push_back(band_waveforms.clone());
                while self.persistence_history.len() > self.persistence_window {
                    self.persistence_history.pop_front();
                }

                let total_frames = self.persistence_history.len().max(1) as f64;
                let fade_steps = (total_frames - 1.0).max(1.0);

                for (age, snapshot) in self.persistence_history.iter().enumerate() {
                    let intensity = 0.2 + 0.8 * ((age as f64) / fade_steps);
                    for ((name, color), waveform) in band_names
                        .iter()
                        .zip(band_colors.iter())
                        .zip(snapshot.iter())
                    {
                        let faded_color = fade_color(*color, intensity);
                        series.push(Series {
                            name: format!("{} #{}", name, age + 1),
                            color: faded_color,
                            samples: waveform.clone(),
                        });
                    }
                }
            }
            VisualStyle::Line => {
                self.persistence_history.clear();

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
            }
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
                            Theme::Doki => Theme::Pinboard,
                            Theme::Pinboard => Theme::Original,
                        };
                    }
                    crossterm::event::KeyCode::Char('b') => {
                        self.background_idx = (self.background_idx + 1) % BACKGROUND_PRESETS.len();
                    }
                    crossterm::event::KeyCode::Char('c') => {
                        self.rgb_wave = !self.rgb_wave;
                    }
                    crossterm::event::KeyCode::Char('v') => {
                        self.cycle_visual_style();
                    }
                    crossterm::event::KeyCode::Char('w') => {
                        self.cycle_wave_count();
                    }
                    crossterm::event::KeyCode::Char('{') => {
                        self.persistence_window = self.persistence_window.saturating_sub(1).max(1);
                    }
                    crossterm::event::KeyCode::Char('}') => {
                        self.persistence_window += 1;
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
        AudioRenderer::new(
            args.sample_rate,
            args.theme,
            VisualStyle::from(args.visual_mode),
            args.persistence_window,
            args.rgb_wave,
            args.rgb_speed,
            args.wave_count,
        ),
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
