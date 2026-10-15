use clap::{Parser, ValueEnum};
use ratatui::style::Color;
use scope_core::render::{rgb_at_time, shade_color, Palette};
use scope_core::{run_app, AppConfig, Matrix, Renderer, Series, VisualStyle};

mod audio;
use crate::audio::AudioSource;
#[cfg(feature = "audio")]
mod dsp;
#[cfg(feature = "audio")]
mod media;

#[cfg(feature = "audio")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum View {
    #[default]
    Wave,
    OceanMist,
    Spectrum,
    Spectrogram,
    Lissajous,
    Persistence,
}

#[cfg(feature = "audio")]
impl View {
    fn next(self) -> Self {
        match self {
            Self::Wave => Self::OceanMist,
            Self::OceanMist => Self::Spectrum,
            Self::Spectrum => Self::Spectrogram,
            Self::Spectrogram => Self::Lissajous,
            Self::Lissajous => Self::Persistence,
            Self::Persistence => Self::Wave,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Wave => "wave",
            Self::OceanMist => "ocean-mist",
            Self::Spectrum => "spectrum",
            Self::Spectrogram => "spectrogram",
            Self::Lissajous => "lissajous",
            Self::Persistence => "persistence",
        }
    }
}

#[cfg(feature = "audio")]
const SPECTROGRAM_ROWS: usize = 120;

#[cfg(feature = "audio")]
const DEFAULT_PERSISTENCE_WINDOW: usize = 12;
#[cfg(feature = "audio")]
const MAX_PERSISTENCE_WINDOW: usize = 120;

#[cfg(feature = "audio")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Theme {
    /// original color defaults used by the initial 3-band visualizer build
    Original,
    /// One saturated hue cycling slowly over 45 seconds; level controls brightness
    Rgb,
    /// Soft monochrome grayscale
    Mono,
}

#[cfg(feature = "audio")]
impl Theme {
    fn colors(&self, elapsed: std::time::Duration) -> [Color; 3] {
        match self {
            Theme::Original => [Color::Cyan, Color::Green, Color::Blue],
            Theme::Rgb => {
                let hue = rgb_at_time(elapsed);
                [hue, shade_color(hue, 0.75), shade_color(hue, 0.5)]
            }
            Theme::Mono => [Color::Gray, Color::DarkGray, Color::White],
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Theme::Original => "original",
            Theme::Rgb => "rgb",
            Theme::Mono => "mono",
        }
    }

    fn palette(self) -> Palette {
        match self {
            Self::Original => Palette::Original,
            Self::Rgb => Palette::Rgb,
            Self::Mono => Palette::Mono,
        }
    }
}

#[cfg(feature = "audio")]
const BACKGROUND_PRESETS: [(Color, &str); 3] = [
    (Color::Reset, "terminal"),
    (Color::Black, "black"),
    (Color::Rgb(0, 8, 24), "midnight"),
];

#[cfg(feature = "audio")]
fn theme_background_idx(theme: Theme) -> usize {
    match theme {
        Theme::Original | Theme::Rgb => 0,
        Theme::Mono => 1,
    }
}

#[cfg(feature = "audio")]
#[derive(Parser)]
#[command(name = "audio-scope")]
struct Cli {
    /// PipeWire sample rate (CPAL uses the device's default configuration)
    #[arg(short = 'r', long, default_value_t = 44_100)]
    sample_rate: u32,
    /// Capture channels (1-2; stereo preferred, native mono remains mono)
    #[arg(short = 'c', long, default_value_t = 2)]
    channels: usize,
    /// Device name: PipeWire target on Linux, exact CPAL input name with --safe or elsewhere
    ///
    /// `auto` uses Linux system audio; other platforms use the default input, not playback.
    #[arg(short = 'd', long, default_value = "auto")]
    device: String,
    /// Use CPAL input capture instead of Linux system audio (already default elsewhere)
    #[arg(long, default_value_t = false)]
    safe: bool,
    /// Visualization theme
    #[arg(short = 't', long, value_enum, default_value_t = Theme::Original)]
    theme: Theme,
    /// Print input device list and exit
    #[arg(short = 'l', long)]
    list_devices: bool,
    /// Ring history
    #[arg(short = 'n', long, default_value_t = 4096)]
    history: usize,
    #[arg(short = 'w', long, default_value_t = 512)]
    width: usize,
    /// Half frame interval in ms (16 => ~32 ms frames; zero gives a 1 ms minimum)
    #[arg(short = 's', long, default_value_t = 16)]
    sleep_ms: u64,
    /// Audio visualization (also cycle with v)
    #[arg(long, value_enum, default_value_t = View::Wave)]
    view: View,
    /// Retained waveform frames in Persistence (1-120; also adjust with { and })
    #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u16).range(1..=120))]
    persistence_window: u16,
    /// Render synthetic audio and poll live media, without opening audio capture
    #[arg(long)]
    smoke: bool,
}

#[cfg(feature = "audio")]
struct AudioRenderer {
    dsp: dsp::Dsp,
    view: View,
    last_view: View,
    media: Option<media::MediaPoller>,
    media_line: String,
    theme: Theme,
    background_idx: usize,
    wave_count: usize,
    persistence_window: usize,
    persistence_bands: usize,
    peak: f64,
    rms: f64,
    clip_pct: f64,
    bass: f64,
    mid: f64,
    treble: f64,
    started: std::time::Instant,
    max_frequency: f64,
    peak_levels: [f64; 64],
    peak_until: [std::time::Duration; 64],
    peak_updated: std::time::Duration,
    mono: Vec<f64>,
}

#[cfg(feature = "audio")]
impl AudioRenderer {
    fn new(sample_rate: u32, theme: Theme, view: View, media: Option<media::MediaPoller>) -> Self {
        Self {
            dsp: dsp::Dsp::new(sample_rate),
            view,
            last_view: view,
            media,
            media_line: "No media player".to_string(),
            theme,
            background_idx: theme_background_idx(theme),
            wave_count: 3,
            persistence_window: DEFAULT_PERSISTENCE_WINDOW,
            persistence_bands: 0,
            peak: 0.0,
            rms: 0.0,
            clip_pct: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            started: std::time::Instant::now(),
            max_frequency: f64::from(sample_rate) / 2.0,
            peak_levels: [0.0; 64],
            peak_until: [std::time::Duration::ZERO; 64],
            peak_updated: std::time::Duration::ZERO,
            mono: Vec::new(),
        }
    }

    fn set_persistence_window(&mut self, window: usize) {
        self.persistence_window = window.clamp(1, MAX_PERSISTENCE_WINDOW);
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

    fn sample_stats(samples: &[f64]) -> (f64, f64, f64) {
        if samples.is_empty() {
            return (0.0, 0.0, 0.0);
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
        (
            peak,
            (sum_sq / samples.len() as f64).sqrt(),
            clipped as f64 * 100.0 / samples.len() as f64,
        )
    }

    fn update_peaks(&mut self, elapsed: std::time::Duration) {
        for ((peak, until), &level) in self
            .peak_levels
            .iter_mut()
            .zip(&mut self.peak_until)
            .zip(&self.dsp.spectrum)
        {
            let decay_start = self.peak_updated.max(*until);
            *peak = (*peak - elapsed.saturating_sub(decay_start).as_secs_f64() * 0.35).max(0.0);
            if level >= *peak {
                *peak = level;
                *until = elapsed + std::time::Duration::from_millis(600);
            }
        }
        self.peak_updated = elapsed;
    }

    fn process_at(
        &mut self,
        cfg: &AppConfig,
        frame: &Matrix,
        series: &mut Vec<Series>,
        elapsed: std::time::Duration,
    ) {
        if let Some(media) = &mut self.media {
            let cached = media.cached();
            if self.media_line != cached {
                self.media_line.clear();
                self.media_line.push_str(cached);
            }
        }
        if self.last_view != self.view {
            series.clear();
            self.persistence_bands = 0;
            self.peak_levels.fill(0.0);
            self.peak_until.fill(elapsed);
            self.peak_updated = elapsed;
            self.last_view = self.view;
        }
        let colors = self.theme.colors(elapsed);
        let palette = self.theme.palette();
        // Persistence applies both band brightness and age after adding the current frame.
        if self.view != View::Persistence {
            // Recolor all retained history using this frame's one global hue.
            for row in series.iter_mut() {
                row.palette = palette;
                row.color = colors[0];
            }
        }
        if self.view == View::Lissajous {
            self.process_xy(cfg, frame, series, colors[0], palette);
            return;
        }
        let analysis_limit = match self.view {
            View::Wave | View::OceanMist | View::Persistence => cfg.width,
            _ => dsp::MAX_FFT.max(cfg.width),
        };
        downmix_frame(frame, analysis_limit, &mut self.mono);
        let row = self.mono.as_slice();
        let waveform_samples = &row[row.len().saturating_sub(cfg.width)..];
        (self.peak, self.rms, self.clip_pct) = Self::sample_stats(waveform_samples);
        let make_waves = matches!(self.view, View::Wave | View::OceanMist | View::Persistence);
        let analysis_samples = if make_waves {
            waveform_samples
        } else {
            &row[row.len().saturating_sub(dsp::MAX_FFT)..]
        };
        if self.view == View::Persistence
            && (analysis_samples.is_empty()
                || analysis_samples.iter().any(|sample| !sample.is_finite()))
        {
            series.clear();
            self.persistence_bands = 0;
            self.dsp.analyze(&[], self.wave_count, true);
            [self.bass, self.mid, self.treble] = self.dsp.levels;
            (self.peak, self.rms, self.clip_pct) = (0.0, 0.0, 0.0);
            return;
        }
        self.dsp
            .analyze(analysis_samples, self.wave_count, make_waves);
        [self.bass, self.mid, self.treble] = self.dsp.levels;
        if self.view == View::Spectrum {
            self.update_peaks(elapsed);
        }
        match self.view {
            View::Persistence => self.process_persistence(series, colors, palette),
            View::Wave | View::OceanMist => {
                let names: &[&str] = match self.wave_count {
                    1 => &["wave"],
                    2 => &["low", "high"],
                    _ => &["bass", "mid", "treble"],
                };
                series.truncate(names.len());
                for (idx, name) in names.iter().enumerate() {
                    if idx == series.len() {
                        series.push(Series {
                            name: String::new(),
                            color: colors[idx],
                            palette,
                            samples: Vec::new(),
                            xy_points: Vec::new(),
                            peak_levels: Vec::new(),
                            max_frequency: self.max_frequency,
                        });
                    }
                    let band = &mut series[idx];
                    if band.name != *name {
                        band.name.clear();
                        band.name.push_str(name);
                    }
                    band.color = colors[idx];
                    if self.view == View::OceanMist {
                        band.peak_levels.clear();
                        band.peak_levels.extend_from_slice(&band.samples);
                    } else {
                        band.peak_levels.clear();
                    }
                    // Swap with the previous output so both sides retain reusable capacity.
                    std::mem::swap(&mut band.samples, &mut self.dsp.waves[idx]);
                }
            }
            View::Spectrum | View::Spectrogram => {
                let cap = if self.view == View::Spectrum {
                    1
                } else {
                    SPECTROGRAM_ROWS
                };
                if series.len() == cap {
                    series.rotate_left(1);
                } else {
                    series.push(Series {
                        name: "20 Hz → Nyquist".to_string(),
                        color: colors[0],
                        palette,
                        samples: Vec::with_capacity(self.dsp.spectrum.len()),
                        xy_points: Vec::new(),
                        peak_levels: Vec::new(),
                        max_frequency: self.max_frequency,
                    });
                }
                if let Some(latest) = series.last_mut() {
                    latest.color = colors[0];
                    latest.samples.clear();
                    latest.samples.extend_from_slice(&self.dsp.spectrum);
                    if self.view == View::Spectrum {
                        latest.peak_levels.clear();
                        latest.peak_levels.extend_from_slice(&self.peak_levels);
                    }
                }
            }
            View::Lissajous => unreachable!("X-Y frames return before mono processing"),
        }
    }

    fn process_persistence(
        &mut self,
        series: &mut Vec<Series>,
        colors: [Color; 3],
        palette: Palette,
    ) {
        let names: &[&str] = match self.wave_count {
            1 => &["wave"],
            2 => &["low", "high"],
            _ => &["bass", "mid", "treble"],
        };
        let bands = names.len();
        if self.persistence_bands != bands {
            series.clear();
            self.persistence_bands = bands;
        }
        let cap = self.persistence_window * bands;
        if series.len() > cap {
            // Keep the newest complete frames before reusing their oldest slot below.
            let discarded = series.len() - cap;
            series.rotate_left(discarded);
            series.truncate(cap);
        }
        if series.len() == cap {
            series.rotate_left(bands);
        } else {
            for _ in 0..bands {
                series.push(Series {
                    name: String::new(),
                    color: colors[0],
                    palette,
                    samples: Vec::new(),
                    xy_points: Vec::new(),
                    peak_levels: Vec::new(),
                    max_frequency: self.max_frequency,
                });
            }
        }
        let newest = series.len() - bands;
        for (band, samples) in series[newest..].iter_mut().zip(&mut self.dsp.waves) {
            // The evicted sample storage becomes the next DSP output buffer.
            std::mem::swap(&mut band.samples, samples);
        }
        // Named ANSI colors need fixed true-color equivalents to fade, too.
        let colors = colors.map(|color| match color {
            Color::Cyan => Color::Rgb(0, 128, 128),
            Color::Green => Color::Rgb(0, 128, 0),
            Color::Blue => Color::Rgb(0, 0, 128),
            Color::Gray => Color::Rgb(192, 192, 192),
            Color::DarkGray => Color::Rgb(128, 128, 128),
            Color::White => Color::Rgb(255, 255, 255),
            color => color,
        });
        let frames = series.len() / bands;
        for (age, frame) in series.chunks_exact_mut(bands).enumerate() {
            let brightness = if frames == 1 {
                1.0
            } else {
                0.2 + 0.8 * age as f64 / (frames - 1) as f64
            };
            for (index, band) in frame.iter_mut().enumerate() {
                band.color = shade_color(colors[index], brightness);
                band.palette = palette;
                let name = if age + 1 == frames { names[index] } else { "" };
                if band.name != name {
                    band.name.clear();
                    band.name.push_str(name);
                }
            }
        }
    }

    fn process_xy(
        &self,
        cfg: &AppConfig,
        frame: &Matrix,
        series: &mut Vec<Series>,
        color: Color,
        palette: Palette,
    ) {
        series.truncate(1);
        if series.is_empty() {
            series.push(Series {
                name: String::new(),
                color,
                palette,
                samples: Vec::new(),
                xy_points: Vec::new(),
                peak_levels: Vec::new(),
                max_frequency: self.max_frequency,
            });
        }
        let trace = &mut series[0];
        trace.color = color;
        trace.palette = palette;
        trace.xy_points.clear();
        let message = match frame.as_slice() {
            [left, right] if left.len() == right.len() => {
                let start = left.len().saturating_sub(cfg.width);
                trace.xy_points.extend(
                    left[start..]
                        .iter()
                        .copied()
                        .zip(right[start..].iter().copied())
                        .filter(|(x, y)| x.is_finite() && y.is_finite()),
                );
                if trace.xy_points.is_empty() {
                    "Awaiting stereo samples"
                } else {
                    "X: left / Y: right"
                }
            }
            [_] => "Stereo X-Y unavailable: mono capture",
            [_, _] => "Stereo X-Y unavailable: channel windows are not aligned",
            _ => "Stereo X-Y unavailable: capture needs exactly two channels",
        };
        if trace.name != message {
            trace.name.clear();
            trace.name.push_str(message);
        }
    }
}

#[cfg(feature = "audio")]
impl Renderer for AudioRenderer {
    fn header(&self) -> String {
        if self.view == View::Lissajous {
            return format!(
                "{}\ntheme {} | bg {} | view lissajous | X left / Y right",
                self.media_line,
                self.theme.title(),
                self.background_name(),
            );
        }
        if self.view == View::Persistence {
            return format!(
                "{}\ntheme {} | bg {} | view persistence | window {} | waves {} | bass {:.2} mid {:.2} treble {:.2} | peak {:.2} rms {:.2} clip {:>3.0}%",
                self.media_line,
                self.theme.title(),
                self.background_name(),
                self.persistence_window,
                self.wave_count,
                self.bass,
                self.mid,
                self.treble,
                self.peak,
                self.rms,
                self.clip_pct,
            );
        }
        let (trace_label, trace_count) = match self.view {
            View::OceanMist => ("layers", self.wave_count),
            _ => ("waves", self.wave_count),
        };
        format!(
            "{}\ntheme {:<7} | bg {:<8} | view {} | {} {} | bass {:.2} mid {:.2} treble {:.2} | peak {:.2} rms {:.2} clip {:>3.0}%",
            self.media_line,
            self.theme.title(),
            self.background_name(),
            self.view.name(),
            trace_label,
            trace_count,
            self.bass,
            self.mid,
            self.treble,
            self.peak,
            self.rms,
            self.clip_pct
        )
    }

    fn y_bounds(&self, _cfg: &AppConfig) -> (f64, f64) {
        match self.view {
            View::Wave | View::Lissajous | View::Persistence => (-1.0, 1.0),
            View::OceanMist | View::Spectrum | View::Spectrogram => (0.0, 1.0),
        }
    }

    fn visual_style(&self) -> VisualStyle {
        match self.view {
            View::Wave => VisualStyle::Line,
            View::OceanMist => VisualStyle::OceanMist,
            View::Spectrum => VisualStyle::Bars,
            View::Spectrogram => VisualStyle::Spectrogram,
            View::Lissajous => VisualStyle::Lissajous,
            View::Persistence => VisualStyle::Persistence,
        }
    }

    fn background_color(&self) -> Color {
        BACKGROUND_PRESETS[self.background_idx % BACKGROUND_PRESETS.len()].0
    }

    fn process(&mut self, cfg: &AppConfig, frame: &Matrix, series: &mut Vec<Series>) {
        self.process_at(cfg, frame, series, self.started.elapsed());
    }

    fn handle_event(&mut self, event: &crossterm::event::Event, _cfg: &mut AppConfig) {
        if let crossterm::event::Event::Key(key) = event {
            if key.kind == crossterm::event::KeyEventKind::Press {
                match key.code {
                    crossterm::event::KeyCode::Char('t') => {
                        self.theme = match self.theme {
                            Theme::Original => Theme::Rgb,
                            Theme::Rgb => Theme::Mono,
                            Theme::Mono => Theme::Original,
                        };
                    }
                    crossterm::event::KeyCode::Char('b') => {
                        self.background_idx = (self.background_idx + 1) % BACKGROUND_PRESETS.len();
                    }
                    crossterm::event::KeyCode::Char('w') => {
                        self.cycle_wave_count();
                    }
                    crossterm::event::KeyCode::Char('v') => {
                        self.view = self.view.next();
                    }
                    crossterm::event::KeyCode::Char('{') => {
                        self.set_persistence_window(self.persistence_window.saturating_sub(1));
                    }
                    crossterm::event::KeyCode::Char('}') => {
                        self.set_persistence_window(self.persistence_window.saturating_add(1));
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Legacy views intentionally average complete channel frames, clamping only after mixing.
#[cfg(feature = "audio")]
fn downmix_frame(frame: &Matrix, limit: usize, mono: &mut Vec<f64>) {
    mono.clear();
    let Some(first) = frame.first() else {
        return;
    };
    if frame.iter().any(|channel| channel.len() != first.len()) {
        return;
    }
    let start = first.len().saturating_sub(limit);
    mono.extend((start..first.len()).map(|index| {
        (frame.iter().map(|channel| channel[index]).sum::<f64>() / frame.len() as f64)
            .clamp(-1.0, 1.0)
    }));
}

#[cfg(feature = "audio")]
fn main() {
    let args = Cli::parse();
    if args.smoke {
        if let Err(err) = smoke(args.theme, args.persistence_window) {
            eprintln!("smoke failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    let system_audio = cfg!(target_os = "linux") && !args.safe;

    if args.list_devices {
        if let Err(err) = AudioSource::list_input_devices(system_audio) {
            eprintln!("could not list devices: {err}");
        }
        return;
    }

    if system_audio {
        eprintln!(
            "system-audio mode active: this will capture playback/monitor sources by default"
        );
    } else {
        eprintln!(
            "input-device mode active: playback capture requires a routed loopback input (e.g. BlackHole on macOS)"
        );
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
        system_audio,
    ) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("audio source init failed: {err}");
            return;
        }
    };

    let sample_rate = source.sample_rate();
    if args.view == View::Lissajous && source.channels() != 2 {
        eprintln!(
            "Stereo X-Y unavailable: this source provides mono; other views remain available"
        );
    }
    let mut renderer = AudioRenderer::new(
        sample_rate,
        args.theme,
        args.view,
        Some(media::MediaPoller::new()),
    );
    renderer.set_persistence_window(usize::from(args.persistence_window));
    if let Err(err) = run_app(source, renderer, config) {
        eprintln!("audio-scope failed: {err:?}");
    }
}

#[cfg(feature = "audio")]
fn trace_rgb_foregrounds(
    buffer: &ratatui::buffer::Buffer,
) -> std::collections::BTreeSet<(u8, u8, u8)> {
    buffer
        .content
        .iter()
        .filter(|cell| {
            cell.symbol()
                .chars()
                .any(|symbol| ('\u{2801}'..='\u{28ff}').contains(&symbol) || symbol == '•')
        })
        .filter_map(|cell| match cell.fg {
            Color::Rgb(r, g, b) if r != g || g != b => Some((r, g, b)),
            _ => None,
        })
        .collect()
}

#[cfg(feature = "audio")]
fn smoke(theme: Theme, persistence_window: u16) -> anyhow::Result<()> {
    use ratatui::{backend::TestBackend, Terminal};
    use std::fmt::Write as _;

    let cfg = AppConfig {
        width: 1024,
        ..AppConfig::default()
    };
    let mut renderer =
        AudioRenderer::new(44_100, theme, View::Wave, Some(media::MediaPoller::new()));
    renderer.set_persistence_window(usize::from(persistence_window));
    renderer.media_line = media::query_once();
    println!("MPRIS: {}", renderer.media_line);
    let mut terminal = Terminal::new(TestBackend::new(96, 18))?;
    let mut series = Vec::new();
    for (view, wave_count) in [
        (View::Wave, 1),
        (View::Wave, 2),
        (View::Wave, 3),
        (View::OceanMist, 3),
        (View::Spectrum, 3),
        (View::Spectrogram, 3),
        (View::Lissajous, 1),
        (View::Persistence, 1),
        (View::Persistence, 2),
        (View::Persistence, 3),
    ] {
        // Exercise the real worker cache over more than one polling interval.
        std::thread::sleep(std::time::Duration::from_millis(600));
        renderer.view = view;
        renderer.wave_count = wave_count;
        let ticks = if view == View::Persistence {
            renderer.persistence_window + 2
        } else {
            16
        };
        let mut previous_current = [0u64; 3];
        let fingerprint = |samples: &[f64]| {
            samples
                .iter()
                .fold(0u64, |hash, sample| hash.rotate_left(7) ^ sample.to_bits())
        };
        for tick in 0..ticks {
            let amplitude = if tick < 8 { 0.5 } else { 0.2 };
            let left: Vec<_> = (0..1024)
                .map(|i| {
                    let time = i as f64 / 44_100.0;
                    amplitude * (std::f64::consts::TAU * (220.0 + tick as f64 * 90.0) * time).sin()
                        + 0.1 * (std::f64::consts::TAU * 4_000.0 * time).sin()
                })
                .collect();
            let frame = if view == View::Lissajous {
                let right = (0..1024)
                    .map(|i| {
                        amplitude
                            * (std::f64::consts::TAU * (220.0 + tick as f64 * 90.0) * i as f64
                                / 44_100.0)
                                .cos()
                    })
                    .collect();
                vec![left, right]
            } else {
                vec![left]
            };
            renderer.process_at(
                &cfg,
                &frame,
                &mut series,
                std::time::Duration::from_secs_f64(12.0 + tick as f64 * 0.05),
            );
            if view == View::Persistence {
                let frames = (tick + 1).min(renderer.persistence_window);
                anyhow::ensure!(
                    series.len() == frames * wave_count,
                    "persistence history length"
                );
                if frames > 1 {
                    let prior = series.len() - 2 * wave_count;
                    for (band, expected) in series[prior..prior + wave_count]
                        .iter()
                        .zip(previous_current)
                    {
                        anyhow::ensure!(
                            fingerprint(&band.samples) == expected,
                            "persistence chronological trail"
                        );
                    }
                }
                for (hash, band) in previous_current
                    .iter_mut()
                    .zip(&series[series.len() - wave_count..])
                {
                    *hash = fingerprint(&band.samples);
                }
                let base = match theme {
                    Theme::Original => [
                        Color::Rgb(0, 128, 128),
                        Color::Rgb(0, 128, 0),
                        Color::Rgb(0, 0, 128),
                    ],
                    Theme::Mono => [
                        Color::Rgb(192, 192, 192),
                        Color::Rgb(128, 128, 128),
                        Color::Rgb(255, 255, 255),
                    ],
                    Theme::Rgb => theme.colors(std::time::Duration::from_secs_f64(
                        12.0 + tick as f64 * 0.05,
                    )),
                };
                for (index, band) in series.iter().enumerate() {
                    let age = index / wave_count;
                    let fade = if frames == 1 {
                        1.0
                    } else {
                        0.2 + 0.8 * age as f64 / (frames - 1) as f64
                    };
                    anyhow::ensure!(
                        band.color == shade_color(base[index % wave_count], fade),
                        "persistence age intensity"
                    );
                    anyhow::ensure!(
                        band.name.is_empty() == (age + 1 != frames),
                        "persistence current legend"
                    );
                }
            }
        }
        let (low, high) = renderer.y_bounds(&cfg);
        scope_core::render::draw_frame(
            &mut terminal,
            &series,
            &cfg,
            &renderer.header(),
            (low, high),
            renderer.background_color(),
            renderer.visual_style(),
        )?;
        let buffer = terminal.backend().buffer();
        let waveform_colors = trace_rgb_foregrounds(buffer);
        if theme == Theme::Rgb
            && matches!(
                view,
                View::Wave | View::OceanMist | View::Lissajous | View::Persistence
            )
        {
            anyhow::ensure!(
                !waveform_colors.is_empty(),
                "RGB trace foreground failure: view={} waves={wave_count} has no RGB strokes in TestBackend",
                view.name()
            );
        }
        if view == View::Persistence {
            let has_stroke = buffer.content.iter().any(|cell| {
                cell.symbol()
                    .chars()
                    .any(|symbol| ('\u{2801}'..='\u{28ff}').contains(&symbol))
                    && matches!(cell.fg, Color::Rgb(r, g, b) if r != 0 || g != 0 || b != 0)
            });
            anyhow::ensure!(has_stroke, "persistence has no visible true-color strokes");
            if theme != Theme::Mono {
                anyhow::ensure!(
                    !waveform_colors.is_empty(),
                    "persistence has no colored strokes"
                );
            }
        }
        println!(
            "\n{} waves={wave_count} theme={} (96×18; {} history rows)",
            view.name(),
            theme.title(),
            series.len()
        );
        for y in 0..18 {
            let mut line = String::new();
            for x in 0..96 {
                if let Some(cell) = buffer.cell((x, y)) {
                    // Serialize the palette's named and true-color cell styles without capture.
                    if let Color::Rgb(r, g, b) = cell.bg {
                        write!(line, "\x1b[48;2;{r};{g};{b}m")?;
                    } else if cell.bg == Color::Black {
                        line.push_str("\x1b[40m");
                    } else {
                        line.push_str("\x1b[49m");
                    }
                    if let Color::Rgb(r, g, b) = cell.fg {
                        write!(line, "\x1b[38;2;{r};{g};{b}m")?;
                    } else {
                        let code = match cell.fg {
                            Color::Green => 32,
                            Color::Blue => 34,
                            Color::Cyan => 36,
                            Color::Gray => 37,
                            Color::DarkGray => 90,
                            Color::White => 97,
                            _ => 39,
                        };
                        write!(line, "\x1b[{code}m")?;
                    }
                    line.push_str(cell.symbol());
                }
            }
            println!("{line}\x1b[0m");
        }
        let heat_cells = buffer
            .content
            .iter()
            .filter(|cell| matches!(cell.bg, Color::Rgb(..)))
            .count();
        let mut foreground_colors = std::collections::BTreeSet::new();
        let mut background_colors = std::collections::BTreeSet::new();
        for cell in &buffer.content {
            if let Color::Rgb(r, g, b) = cell.fg {
                foreground_colors.insert((r, g, b));
            }
            if let Color::Rgb(r, g, b) = cell.bg {
                background_colors.insert((r, g, b));
            }
        }
        if theme == Theme::Rgb && view == View::Spectrum {
            anyhow::ensure!(
                foreground_colors.iter().any(|&(r, g, b)| r != g || g != b),
                "RGB spectrum failure: no colored bar foregrounds in TestBackend"
            );
        }
        if theme == Theme::Rgb && view == View::Spectrogram {
            anyhow::ensure!(
                background_colors.iter().any(|&(r, g, b)| {
                    (r != g || g != b) && Color::Rgb(r, g, b) != renderer.background_color()
                }),
                "RGB spectrogram failure: no colored signal backgrounds in TestBackend"
            );
        }
        let foreground_count = if matches!(
            view,
            View::Wave | View::OceanMist | View::Lissajous | View::Persistence
        ) {
            waveform_colors.len()
        } else {
            foreground_colors.len()
        };
        if view == View::Lissajous {
            println!(
                "stereo_pairs={} rgb_foreground_colors={} rgb_background_colors={}",
                series[0].xy_points.len(),
                foreground_count,
                background_colors.len()
            );
        } else {
            println!(
                "peak={:.3} rms={:.3} heatmap_cells={heat_cells} rgb_foreground_colors={} rgb_background_colors={}",
                renderer.peak, renderer.rms, foreground_count, background_colors.len()
            );
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "audio"))]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn tone(freq: f64) -> Matrix {
        vec![(0..1024)
            .map(|i| 0.5 * (std::f64::consts::TAU * freq * i as f64 / 44_100.0).sin())
            .collect()]
    }

    fn peak_bin(samples: &[f64]) -> usize {
        samples
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(idx, _)| idx)
            .unwrap_or(0)
    }

    #[test]
    fn persistence_evicts_chronologically_and_fades_oldest_to_newest() {
        let cfg = AppConfig {
            width: 4,
            ..Default::default()
        };
        for window in [1, 3] {
            let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Persistence, None);
            renderer.wave_count = 1;
            renderer.set_persistence_window(window);
            let mut series = Vec::new();
            for tick in 1usize..=7 {
                let value = tick as f64 / 10.0;
                renderer.process_at(
                    &cfg,
                    &vec![vec![value; 8]],
                    &mut series,
                    std::time::Duration::ZERO,
                );
                let frames = tick.min(window);
                assert_eq!(series.len(), frames);
                for (index, band) in series.iter().enumerate() {
                    assert_eq!(
                        band.samples,
                        vec![(tick - frames + index + 1) as f64 / 10.0; 4]
                    );
                    let fade = if frames == 1 {
                        1.0
                    } else {
                        0.2 + 0.8 * index as f64 / (frames - 1) as f64
                    };
                    assert_eq!(band.color, shade_color(Color::Rgb(0, 128, 128), fade));
                    assert_eq!(band.name, if index + 1 == frames { "wave" } else { "" });
                }
            }
        }
    }

    #[test]
    fn persistence_window_bounds_and_resize_retain_newest_frames() {
        for value in ["1", "12", "120"] {
            let args = Cli::try_parse_from([
                "audio-scope",
                "--view",
                "persistence",
                "--persistence-window",
                value,
            ])
            .expect("valid persistence window");
            assert_eq!(args.view, View::Persistence);
            assert_eq!(args.persistence_window.to_string(), value);
        }
        for value in ["0", "121", "65536", "-1"] {
            assert!(Cli::try_parse_from(["audio-scope", "--persistence-window", value]).is_err());
        }
        let mut cfg = AppConfig {
            width: 4,
            ..Default::default()
        };
        let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Persistence, None);
        renderer.wave_count = 1;
        let mut series = Vec::new();
        renderer.set_persistence_window(4);
        for tick in 1..=5 {
            renderer.process(&cfg, &vec![vec![tick as f64 / 10.0; 4]], &mut series);
        }
        renderer.set_persistence_window(2);
        renderer.process(&cfg, &vec![vec![0.6; 4]], &mut series);
        assert_eq!(
            series
                .iter()
                .map(|band| band.samples[0])
                .collect::<Vec<_>>(),
            [0.5, 0.6]
        );
        renderer.set_persistence_window(4);
        for value in [0.7, 0.8] {
            renderer.process(&cfg, &vec![vec![value; 4]], &mut series);
        }
        assert_eq!(
            series
                .iter()
                .map(|band| band.samples[0])
                .collect::<Vec<_>>(),
            [0.5, 0.6, 0.7, 0.8]
        );
        // A viewport-width change must not discard raw samples in retained frames.
        cfg.width = 2;
        renderer.process(&cfg, &vec![vec![0.9; 4]], &mut series);
        assert_eq!(series[0].samples, [0.6; 4]);
        assert_eq!(series[3].samples, [0.9; 2]);
        for (key, expected) in [('{', 1), ('}', 120)] {
            let event = Event::Key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
            for _ in 0..150 {
                renderer.handle_event(&event, &mut cfg);
            }
            assert_eq!(renderer.persistence_window, expected);
        }
        renderer.set_persistence_window(0);
        assert_eq!(renderer.persistence_window, 1);
        renderer.set_persistence_window(usize::MAX);
        assert_eq!(renderer.persistence_window, 120);
    }

    #[test]
    fn persistence_groups_match_wave_bands_and_clear_on_band_count_changes() {
        let cfg = AppConfig {
            width: 512,
            ..Default::default()
        };
        let mut renderer = AudioRenderer::new(44_100, Theme::Rgb, View::Persistence, None);
        renderer.set_persistence_window(2);
        let mut reference = AudioRenderer::new(44_100, Theme::Rgb, View::Wave, None);
        let mut series = Vec::new();
        let mut reference_series = Vec::new();
        for count in [1, 2, 3, 2, 1, 3] {
            renderer.wave_count = count;
            reference.wave_count = count;
            let mut prior = Vec::new();
            for tick in 0..3 {
                let frame = tone(220.0 + tick as f64 * 1000.0);
                renderer.process_at(&cfg, &frame, &mut series, std::time::Duration::ZERO);
                reference.process_at(
                    &cfg,
                    &frame,
                    &mut reference_series,
                    std::time::Duration::ZERO,
                );
                assert_eq!(series.len(), (tick + 1).min(2) * count);
                if tick > 0 {
                    for (band, samples) in series[..count].iter().zip(&prior) {
                        assert_eq!(&band.samples, samples);
                        assert!(band.name.is_empty());
                    }
                }
                let newest = series.len() - count;
                for (band, expected) in series[newest..].iter().zip(&reference_series) {
                    assert_eq!(band.samples, expected.samples);
                    assert_eq!(band.name, expected.name);
                    assert_eq!(band.color, expected.color);
                }
                prior = reference_series
                    .iter()
                    .map(|band| band.samples.clone())
                    .collect::<Vec<_>>();
            }
        }
    }

    #[test]
    fn persistence_silence_ages_signals_out_and_invalid_downmix_clears_trails() {
        let cfg = AppConfig {
            width: 512,
            ..Default::default()
        };
        for count in 1..=3 {
            let mut renderer = AudioRenderer::new(44_100, Theme::Rgb, View::Persistence, None);
            renderer.wave_count = count;
            renderer.set_persistence_window(3);
            let mut series = Vec::new();
            let silence = vec![vec![0.0; 512]];
            renderer.process(&cfg, &tone(1000.0), &mut series);
            assert!(series
                .iter()
                .any(|band| band.samples.iter().any(|&sample| sample != 0.0)));
            renderer.process(&cfg, &silence, &mut series);
            assert_eq!(series.len(), 2 * count);
            assert!(series[..count]
                .iter()
                .any(|band| band.samples.iter().any(|&sample| sample != 0.0)));
            assert!(series[count..]
                .iter()
                .all(|band| band.samples.iter().all(|&sample| sample == 0.0)));
            for _ in 0..2 {
                renderer.process(&cfg, &silence, &mut series);
            }
            assert_eq!(series.len(), 3 * count);
            assert!(series.iter().all(|band| band.samples.len() == 512
                && band.samples.iter().all(|&sample| sample == 0.0)));
            for invalid in [
                Vec::new(),
                vec![Vec::new()],
                vec![vec![0.5; 512], vec![0.5; 511]],
                vec![vec![f64::NAN; 512]],
            ] {
                renderer.process(&cfg, &tone(1000.0), &mut series);
                renderer.process(&cfg, &invalid, &mut series);
                assert!(series.is_empty(), "invalid downmix retained trails");
                assert_eq!(
                    (renderer.peak, renderer.rms, renderer.clip_pct),
                    (0.0, 0.0, 0.0)
                );
                renderer.process_at(&cfg, &silence, &mut series, std::time::Duration::ZERO);
                assert_eq!(series.len(), count);
                for (index, band) in series.iter().enumerate() {
                    assert_eq!(
                        band.color,
                        Theme::Rgb.colors(std::time::Duration::ZERO)[index]
                    );
                }
            }
        }
    }

    #[test]
    fn persistence_leave_and_reenter_starts_a_fresh_full_intensity_history() {
        let cfg = AppConfig {
            width: 8,
            ..Default::default()
        };
        let mut renderer = AudioRenderer::new(44_100, Theme::Rgb, View::Persistence, None);
        renderer.wave_count = 1;
        let mut series = Vec::new();
        for value in [0.1, 0.2] {
            renderer.process_at(
                &cfg,
                &vec![vec![value; 8]],
                &mut series,
                std::time::Duration::ZERO,
            );
        }
        assert_eq!(series.len(), 2);
        for view in [View::Wave, View::Persistence] {
            renderer.view = view;
            renderer.process_at(
                &cfg,
                &vec![vec![0.3; 8]],
                &mut series,
                std::time::Duration::ZERO,
            );
            assert_eq!(series.len(), 1);
            assert_eq!(series[0].samples, [0.3; 8]);
            assert_eq!(series[0].color, Color::Rgb(255, 0, 0));
        }
    }

    #[test]
    fn persistence_recolors_retained_samples_for_every_theme_and_one_shared_45s_hue() {
        let cfg = AppConfig {
            width: 512,
            ..Default::default()
        };
        let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Persistence, None);
        let mut series = Vec::new();
        renderer.process_at(&cfg, &tone(1000.0), &mut series, std::time::Duration::ZERO);
        let original: Vec<_> = series.iter().map(|band| band.samples.clone()).collect();
        for (theme, millis, base) in [
            (
                Theme::Mono,
                0,
                [
                    Color::Rgb(192, 192, 192),
                    Color::Rgb(128, 128, 128),
                    Color::Rgb(255, 255, 255),
                ],
            ),
            (
                Theme::Original,
                0,
                [
                    Color::Rgb(0, 128, 128),
                    Color::Rgb(0, 128, 0),
                    Color::Rgb(0, 0, 128),
                ],
            ),
            (
                Theme::Rgb,
                3750,
                [
                    Color::Rgb(255, 128, 0),
                    shade_color(Color::Rgb(255, 128, 0), 0.75),
                    shade_color(Color::Rgb(255, 128, 0), 0.5),
                ],
            ),
            (
                Theme::Rgb,
                15000,
                [
                    Color::Rgb(0, 255, 0),
                    shade_color(Color::Rgb(0, 255, 0), 0.75),
                    shade_color(Color::Rgb(0, 255, 0), 0.5),
                ],
            ),
            (
                Theme::Rgb,
                48750,
                [
                    Color::Rgb(255, 128, 0),
                    shade_color(Color::Rgb(255, 128, 0), 0.75),
                    shade_color(Color::Rgb(255, 128, 0), 0.5),
                ],
            ),
        ] {
            renderer.theme = theme;
            renderer.process_at(
                &cfg,
                &tone(2000.0),
                &mut series,
                std::time::Duration::from_millis(millis),
            );
            for (band, expected) in series[..3].iter().zip(&original) {
                assert_eq!(&band.samples, expected, "theme change erased raw history");
            }
            let frames = series.len() / 3;
            for (index, band) in series.iter().enumerate() {
                let fade = 0.2 + 0.8 * (index / 3) as f64 / (frames - 1) as f64;
                assert_eq!(band.color, shade_color(base[index % 3], fade));
                assert_eq!(band.palette, theme.palette());
                if theme == Theme::Rgb {
                    assert_hue(
                        band.color,
                        rgb_at_time(std::time::Duration::from_millis(millis)),
                    );
                }
            }
        }
    }

    #[test]
    fn default_wave_and_view_cycle_preserve_raw_wave_selection() {
        let args = Cli::parse_from(["audio-scope"]);
        assert_eq!(args.view, View::Wave);
        assert_eq!(args.history, 4096);
        assert_eq!(args.persistence_window, 12);
        assert!(Cli::try_parse_from(["audio-scope", "--view", "scope"]).is_err());
        let mut renderer = AudioRenderer::new(44_100, Theme::Original, args.view, None);
        let mut cfg = AppConfig {
            width: 1024,
            ..AppConfig::default()
        };
        let mut series = Vec::new();
        let frame = tone(220.0);
        renderer.wave_count = 1;
        renderer.process(&cfg, &frame, &mut series);
        assert_eq!(series[0].samples, frame[0]);
        let event = Event::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        for expected in [
            View::OceanMist,
            View::Spectrum,
            View::Spectrogram,
            View::Lissajous,
            View::Persistence,
            View::Wave,
        ] {
            renderer.handle_event(&event, &mut cfg);
            assert_eq!(renderer.view, expected);
            renderer.process(&cfg, &frame, &mut series);
        }
        assert_eq!(series[0].samples, frame[0]);
    }

    #[test]
    fn lissajous_preserves_antiphase_stereo_that_cancels_in_mono_views() {
        let cfg = AppConfig {
            width: 32,
            ..Default::default()
        };
        let left: Vec<_> = (0..64)
            .map(|index| [0.75, 0.25, -0.75, -0.25][index % 4])
            .collect();
        let right = left.iter().map(|sample| -sample).collect();
        let frame = vec![left, right];
        let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Wave, None);
        renderer.wave_count = 1;
        let mut series = Vec::new();
        renderer.process(&cfg, &frame, &mut series);
        assert!(series[0].samples.iter().all(|sample| *sample == 0.0));

        renderer.view = View::Lissajous;
        renderer.process(&cfg, &frame, &mut series);
        assert!(series[0].samples.is_empty());
        assert_eq!(series[0].xy_points.len(), 32);
        assert!(series[0]
            .xy_points
            .iter()
            .all(|(left, right)| *right == -*left));
        assert!(series[0].xy_points.contains(&(0.75, -0.75)));
        assert!(series[0].xy_points.contains(&(-0.75, 0.75)));

        renderer.view = View::Spectrum;
        renderer.process(&cfg, &frame, &mut series);
        assert!(series[0].samples.iter().all(|sample| *sample == 0.0));
        assert!(series[0].xy_points.is_empty());
    }

    #[test]
    fn lissajous_uses_one_shared_recent_window_and_rejects_unaligned_or_mono_input() {
        let cfg = AppConfig {
            width: 3,
            ..Default::default()
        };
        let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Lissajous, None);
        let mut series = Vec::new();
        let aligned = vec![
            vec![0.1, 0.2, 0.3, 0.4, 0.5],
            vec![-0.5, -0.4, -0.3, -0.2, -0.1],
        ];
        renderer.process(&cfg, &aligned, &mut series);
        assert_eq!(series[0].xy_points, [(0.3, -0.3), (0.4, -0.2), (0.5, -0.1)]);
        for invalid in [
            vec![aligned[0].clone(), aligned[1][1..].to_vec()],
            vec![aligned[0].clone()],
            vec![Vec::new(), Vec::new()],
            Vec::new(),
        ] {
            renderer.process(&cfg, &invalid, &mut series);
            assert!(
                series[0].xy_points.is_empty(),
                "invalid input must not retain a stale curve"
            );
        }
        renderer.process(&cfg, &aligned, &mut series);
        assert_eq!(series[0].xy_points, [(0.3, -0.3), (0.4, -0.2), (0.5, -0.1)]);
        renderer.process(&cfg, &vec![vec![0.0; 5], vec![0.0; 5]], &mut series);
        assert_eq!(series[0].xy_points, [(0.0, 0.0); 3]);
    }

    #[test]
    fn stereo_legacy_views_match_the_original_clamp_after_average_downmix() {
        let cfg = AppConfig {
            width: 1024,
            ..Default::default()
        };
        let left: Vec<_> = (0..2048)
            .map(|index| [2.0, 2.0, 0.75, -0.5][index % 4])
            .collect();
        let right: Vec<_> = (0..2048)
            .map(|index| [0.0, -2.0, -0.25, 0.0][index % 4])
            .collect();
        let expected = vec![(0..2048)
            .map(|index| [1.0, 0.0, 0.25, -0.25][index % 4])
            .collect()];
        let stereo = vec![left, right];
        for view in [
            View::Wave,
            View::OceanMist,
            View::Spectrum,
            View::Spectrogram,
            View::Persistence,
        ] {
            for count in 1..=3 {
                let mut actual = AudioRenderer::new(44_100, Theme::Original, view, None);
                let mut reference = AudioRenderer::new(44_100, Theme::Original, view, None);
                actual.wave_count = count;
                reference.wave_count = count;
                let mut actual_series = Vec::new();
                let mut reference_series = Vec::new();
                for elapsed in [0, 32] {
                    let elapsed = std::time::Duration::from_millis(elapsed);
                    actual.process_at(&cfg, &stereo, &mut actual_series, elapsed);
                    reference.process_at(&cfg, &expected, &mut reference_series, elapsed);
                    assert_eq!(actual.peak, reference.peak);
                    assert_eq!(actual.rms, reference.rms);
                    assert_eq!(actual.clip_pct, reference.clip_pct);
                    assert_eq!(actual.dsp.levels, reference.dsp.levels);
                    for (actual, reference) in actual_series.iter().zip(&reference_series) {
                        assert_eq!(actual.samples, reference.samples, "{view:?}, waves={count}");
                        assert_eq!(actual.peak_levels, reference.peak_levels);
                    }
                }
            }
        }
    }

    #[test]
    fn ocean_mist_retains_one_dim_previous_trace() {
        let cfg = AppConfig {
            width: 1024,
            ..AppConfig::default()
        };
        let first = tone(220.0);
        let second = tone(440.0);
        let mut renderer = AudioRenderer::new(44_100, Theme::Rgb, View::OceanMist, None);
        renderer.wave_count = 3;
        let mut series = Vec::new();
        renderer.process(&cfg, &first, &mut series);
        assert_eq!(series.len(), 3);
        assert!(series.iter().all(|band| band.peak_levels.is_empty()));
        let prior_layers: Vec<_> = series.iter().map(|band| band.samples.clone()).collect();
        renderer.process(&cfg, &second, &mut series);
        for (band, prior) in series.iter().zip(prior_layers) {
            assert_eq!(band.peak_levels, prior);
        }
    }

    #[test]
    fn spectral_views_analyze_more_samples_than_the_waveform_width() {
        let cfg = AppConfig {
            width: 512,
            ..AppConfig::default()
        };
        let samples: Vec<_> = (0..dsp::MAX_FFT)
            .map(|index| 0.5 * (std::f64::consts::TAU * 60.0 * index as f64 / 44_100.0).sin())
            .collect();
        let frame = vec![samples];
        let mut renderer = AudioRenderer::new(44_100, Theme::Rgb, View::Spectrum, None);
        let mut series = Vec::new();
        renderer.process_at(&cfg, &frame, &mut series, std::time::Duration::ZERO);

        let mut expected = dsp::Dsp::new(44_100);
        expected.analyze(&frame[0], 3, false);
        assert_eq!(series[0].samples, expected.spectrum.to_vec());
        assert!(series[0].samples[..20].iter().any(|&level| level > 0.0));
    }

    #[test]
    fn reacting_spectrum_renders_low_and_high_energy_at_both_horizontal_edges(
    ) -> std::io::Result<()> {
        use ratatui::{backend::TestBackend, Terminal};

        let sample_rate = 44_100.0;
        let cfg = AppConfig {
            width: 512,
            ..AppConfig::default()
        };
        let samples: Vec<_> = (0..dsp::MAX_FFT)
            .map(|index| {
                let time = index as f64 / sample_rate;
                0.2 * (std::f64::consts::TAU * 30.0 * time).sin()
                    + 0.2 * (std::f64::consts::TAU * 20_000.0 * time).sin()
            })
            .collect();
        let frame = vec![samples];
        let mut renderer = AudioRenderer::new(44_100, Theme::Rgb, View::Spectrum, None);
        let mut series = Vec::new();
        renderer.process_at(&cfg, &frame, &mut series, std::time::Duration::ZERO);
        assert!(series[0].samples[0] > 0.0, "low-frequency edge is empty");
        assert!(series[0].samples[63] > 0.0, "high-frequency edge is empty");

        let mut terminal = Terminal::new(TestBackend::new(120, 30))?;
        scope_core::render::draw_frame(
            &mut terminal,
            &series,
            &cfg,
            "",
            (0.0, 1.0),
            Color::Black,
            VisualStyle::Bars,
        )?;
        let buffer = terminal.backend().buffer();
        for col in [0, 119] {
            assert!(
                (0..30).any(|row| matches!(
                    buffer[(col, row)].symbol(),
                    "▁" | "▂" | "▃" | "▄" | "▅" | "▆" | "▇" | "█"
                )),
                "reactive bar missing at terminal column {col}"
            );
        }
        Ok(())
    }

    #[test]
    fn spectrogram_evicts_oldest_and_resets_on_view_change() {
        let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Spectrogram, None);
        let cfg = AppConfig {
            width: 1024,
            ..AppConfig::default()
        };
        let mut series = Vec::new();
        renderer.process(&cfg, &tone(100.0), &mut series);
        let low_bin = peak_bin(&series[0].samples);
        let middle = tone(1_000.0);
        for _ in 1..SPECTROGRAM_ROWS {
            renderer.process(&cfg, &middle, &mut series);
        }
        assert_eq!(series.len(), SPECTROGRAM_ROWS);
        assert_eq!(peak_bin(&series[0].samples), low_bin);
        renderer.process(&cfg, &tone(8_000.0), &mut series);
        assert_eq!(series.len(), SPECTROGRAM_ROWS);
        assert!(peak_bin(&series[0].samples) > low_bin);
        assert!(peak_bin(&series[SPECTROGRAM_ROWS - 1].samples) > peak_bin(&series[0].samples));
        renderer.view = View::Spectrum;
        renderer.process(&cfg, &vec![Vec::new()], &mut series);
        assert_eq!(series.len(), 1);
        assert!(series[0].samples.iter().all(|&level| level == 0.0));
        renderer.view = View::Spectrogram;
        renderer.process(&cfg, &middle, &mut series);
        assert_eq!(series.len(), 1);
        assert!(peak_bin(&series[0].samples) > low_bin);
    }

    #[test]
    fn theme_cli_and_keyboard_cycle_expose_only_original_rgb_and_mono() {
        let defaults = Cli::parse_from(["audio-scope"]);
        assert_eq!(defaults.theme, Theme::Original);
        for (name, theme) in [
            ("original", Theme::Original),
            ("rgb", Theme::Rgb),
            ("mono", Theme::Mono),
        ] {
            assert_eq!(
                Cli::parse_from(["audio-scope", "--theme", name]).theme,
                theme
            );
        }
        for removed in ["classic", "neon", "ocean"] {
            assert!(Cli::try_parse_from(["audio-scope", "--theme", removed]).is_err());
        }
        let mut renderer = AudioRenderer::new(44_100, defaults.theme, View::Wave, None);
        let mut cfg = AppConfig::default();
        let event = Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        for expected in [Theme::Rgb, Theme::Mono, Theme::Original] {
            renderer.handle_event(&event, &mut cfg);
            assert_eq!(renderer.theme, expected);
            assert_eq!(renderer.background_name(), "terminal");
        }
    }

    #[test]
    fn background_key_cycles_only_terminal_black_and_midnight() {
        let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Wave, None);
        let mut cfg = AppConfig::default();
        assert_eq!(renderer.background_color(), Color::Reset);
        let event = Event::Key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        for (name, color) in [
            ("black", Color::Black),
            ("midnight", Color::Rgb(0, 8, 24)),
            ("terminal", Color::Reset),
        ] {
            renderer.handle_event(&event, &mut cfg);
            assert_eq!(renderer.background_name(), name);
            assert_eq!(renderer.background_color(), color);
        }
        let mono = AudioRenderer::new(44_100, Theme::Mono, View::Wave, None);
        assert_eq!(mono.background_color(), Color::Black);
    }

    fn assert_hue(color: Color, base: Color) {
        let (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) = (color, base) else {
            panic!("expected true-color signal");
        };
        let peak = r.max(g).max(b);
        assert!(peak > 0);
        assert_eq!(r.min(g).min(b), 0, "signal hue lost saturation");
        for (actual, expected) in [r, g, b].into_iter().zip([br, bg, bb]) {
            let expected = (f64::from(expected) * f64::from(peak) / 255.0).round() as u8;
            assert!(
                actual.abs_diff(expected) <= 1,
                "spatial hue shift: {color:?} vs {base:?}"
            );
        }
    }

    #[test]
    fn rgb_wave_foregrounds_share_one_hue_for_each_count_with_and_without_header(
    ) -> std::io::Result<()> {
        use ratatui::{backend::TestBackend, Terminal};

        let mut cfg = AppConfig {
            show_ui: false,
            width: 1024,
            ..Default::default()
        };
        let frame = vec![(0..1024)
            .map(|i| {
                [220.0, 1_200.0, 6_000.0]
                    .iter()
                    .map(|frequency| {
                        0.3 * (std::f64::consts::TAU * frequency * i as f64 / 44_100.0).sin()
                    })
                    .sum()
            })
            .collect()];
        let mut renderer = AudioRenderer::new(44_100, Theme::Rgb, View::Wave, None);
        let mut terminal = Terminal::new(TestBackend::new(96, 18))?;
        let mut series = Vec::new();
        for show_ui in [false, true] {
            cfg.show_ui = show_ui;
            for count in 1..=3 {
                renderer.wave_count = count;
                renderer.process_at(
                    &cfg,
                    &frame,
                    &mut series,
                    std::time::Duration::from_millis(3750),
                );
                if count == 1 {
                    assert_eq!(series[0].samples, frame[0]);
                }
                scope_core::render::draw_frame(
                    &mut terminal,
                    &series,
                    &cfg,
                    &renderer.header(),
                    (-1.0, 1.0),
                    renderer.background_color(),
                    renderer.visual_style(),
                )?;
                let colors = trace_rgb_foregrounds(terminal.backend().buffer());
                assert!(
                    !colors.is_empty(),
                    "wave_count={count}, show_ui={show_ui}: no RGB stroke colors"
                );
                assert!(
                    colors.len() <= count,
                    "wave_count={count}: spatial color gradient remains"
                );
                for (r, g, b) in colors {
                    assert_hue(Color::Rgb(r, g, b), Color::Rgb(255, 128, 0));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn injected_time_keeps_one_hue_across_views_and_recolors_all_history() -> std::io::Result<()> {
        use ratatui::{backend::TestBackend, Terminal};

        let cfg = AppConfig {
            show_ui: false,
            width: 1024,
            ..Default::default()
        };
        let mut renderer = AudioRenderer::new(44_100, Theme::Rgb, View::Wave, None);
        let mut terminal = Terminal::new(TestBackend::new(96, 18))?;
        let mut series = Vec::new();
        let mono = tone(1_000.0);
        let frame = vec![mono[0].clone(), mono[0].clone()];
        let mut first_wave = None;
        for (millis, expected) in [
            (3750, Color::Rgb(255, 128, 0)),
            (3850, Color::Rgb(255, 131, 0)),
            (15000, Color::Rgb(0, 255, 0)),
            (48750, Color::Rgb(255, 128, 0)),
        ] {
            for view in [
                View::Wave,
                View::OceanMist,
                View::Spectrum,
                View::Spectrogram,
                View::Lissajous,
                View::Persistence,
            ] {
                renderer.view = view;
                renderer.wave_count = 1;
                // Retained rows must use today's hue, not their capture-time hue.
                if matches!(view, View::Spectrogram | View::Persistence) {
                    renderer.process_at(
                        &cfg,
                        &frame,
                        &mut series,
                        std::time::Duration::from_secs(1),
                    );
                }
                renderer.process_at(
                    &cfg,
                    &frame,
                    &mut series,
                    std::time::Duration::from_millis(millis),
                );
                scope_core::render::draw_frame(
                    &mut terminal,
                    &series,
                    &cfg,
                    "",
                    renderer.y_bounds(&cfg),
                    Color::Black,
                    renderer.visual_style(),
                )?;
                let buffer = terminal.backend().buffer();
                let signal_colors: Vec<_> = buffer
                    .content
                    .iter()
                    .flat_map(|cell| [cell.fg, cell.bg])
                    .filter(|color| matches!(color, Color::Rgb(r,g,b) if r != g || g != b))
                    .collect();
                assert!(!signal_colors.is_empty(), "{view:?}: missing RGB signal");
                for color in signal_colors {
                    assert_hue(color, expected);
                }
                if view == View::Wave {
                    let colors = trace_rgb_foregrounds(buffer);
                    assert_eq!(colors.len(), 1);
                    let color = colors.iter().next().copied();
                    if millis == 3750 {
                        first_wave = color;
                    } else if millis == 3850 {
                        let (Some((r, g, b)), Some((fr, fg, fb))) = (color, first_wave) else {
                            panic!("missing waveform color");
                        };
                        assert_ne!((r, g, b), (fr, fg, fb));
                        assert!(r.abs_diff(fr).max(g.abs_diff(fg)).max(b.abs_diff(fb)) <= 4);
                    } else if millis == 48750 {
                        assert_eq!(color, first_wave, "45-second cycle did not wrap");
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn spectrum_peak_hold_survives_a_drop_then_decays_and_reacquires() -> std::io::Result<()> {
        use ratatui::{backend::TestBackend, Terminal};
        use std::time::Duration;

        let cfg = AppConfig {
            show_ui: false,
            width: 1024,
            ..Default::default()
        };
        let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Spectrum, None);
        let mut series = Vec::new();
        renderer.process_at(&cfg, &tone(1_000.0), &mut series, Duration::ZERO);
        let bin = peak_bin(&series[0].samples);
        let peak = series[0].peak_levels[bin];
        let silence = vec![vec![0.0; 1024]];
        renderer.process_at(&cfg, &silence, &mut series, Duration::from_millis(500));
        assert_eq!(series[0].peak_levels[bin], peak);
        let mut terminal = Terminal::new(TestBackend::new(96, 18))?;
        scope_core::render::draw_frame(
            &mut terminal,
            &series,
            &cfg,
            "",
            (0.0, 1.0),
            Color::Black,
            VisualStyle::Bars,
        )?;
        let buffer = terminal.backend().buffer();
        assert!(
            (0..15).any(|y| (0..96).any(|x| buffer[(x, y)].symbol() == "─")),
            "held peak must appear above the zero baseline"
        );
        renderer.process_at(&cfg, &silence, &mut series, Duration::from_millis(1100));
        assert!((series[0].peak_levels[bin] - (peak - 0.175)).abs() < 1e-9);
        renderer.process_at(&cfg, &silence, &mut series, Duration::from_secs(5));
        assert!(series[0].peak_levels.iter().all(|&level| level == 0.0));
        renderer.process_at(&cfg, &tone(1_000.0), &mut series, Duration::from_secs(6));
        assert_eq!(series[0].peak_levels[bin], peak);
        Ok(())
    }

    #[test]
    fn theme_switch_recolors_retained_spectrogram_without_erasing_history() -> std::io::Result<()> {
        use ratatui::{backend::TestBackend, Terminal};

        let mut renderer = AudioRenderer::new(44_100, Theme::Original, View::Spectrogram, None);
        let mut cfg = AppConfig {
            show_ui: false,
            width: 1024,
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(64, 8))?;
        let mut series = Vec::new();
        let frame = tone(1_000.0);
        renderer.process(&cfg, &frame, &mut series);
        let original_row = series[0].samples.clone();
        let peak = 4 + (peak_bin(&original_row) * 60).div_ceil(original_row.len()) as u16;
        scope_core::render::draw_frame(
            &mut terminal,
            &series,
            &cfg,
            "",
            (0.0, 1.0),
            Color::Black,
            VisualStyle::Spectrogram,
        )?;
        let original_pixel = terminal.backend().buffer()[(peak, 5)].bg;
        let event = Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        for expected in [Palette::Rgb, Palette::Mono, Palette::Original] {
            renderer.handle_event(&event, &mut cfg);
            renderer.process(&cfg, &frame, &mut series);
            assert_eq!(series[0].samples, original_row);
            scope_core::render::draw_frame(
                &mut terminal,
                &series,
                &cfg,
                "",
                (0.0, 1.0),
                Color::Black,
                VisualStyle::Spectrogram,
            )?;
            let pixel = terminal.backend().buffer()[(peak, 6 - series.len() as u16)].bg;
            match (expected, pixel) {
                (Palette::Rgb, Color::Rgb(r, g, b)) => {
                    assert!(r != g || g != b);
                    assert_ne!(pixel, original_pixel);
                }
                (Palette::Mono, Color::Rgb(r, g, b)) => {
                    assert_eq!(r, g);
                    assert_eq!(g, b);
                    assert!(r > 0);
                }
                (Palette::Original, _) => assert_eq!(pixel, original_pixel),
                _ => panic!("spectrogram did not apply the selected palette"),
            }
        }
        Ok(())
    }
}

#[cfg(not(feature = "audio"))]
fn main() {
    eprintln!(
        "audio-scope requires the `audio` feature: cargo run --bin audio-scope --features audio"
    );
}
