#[cfg(feature = "audio")]
use clap::Parser;
#[cfg(feature = "audio")]
use ratatui::style::Color;
#[cfg(feature = "audio")]
use scope_studio::{
    data::audio::AudioSource, render::Series, run_app, AppConfig, Matrix, Renderer,
};

#[cfg(feature = "audio")]
#[derive(Parser)]
#[command(name = "audio-scope")]
struct Cli {
    /// Input sample rate
    #[arg(short = 'r', long, default_value_t = 44_100)]
    sample_rate: u32,
    /// Input channels (1-2)
    #[arg(short = 'c', long, default_value_t = 1)]
    channels: usize,
    /// Input device name (exact match from available input devices)
    ///
    /// Pass `auto` (default) to capture system audio by default.
    #[arg(short = 'd', long, default_value = "auto")]
    device: String,
    /// Prefer safe non-monitor input sources instead of system audio capture
    #[arg(long, default_value_t = false)]
    safe: bool,
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
struct AudioRenderer;

#[cfg(feature = "audio")]
impl AudioRenderer {
    fn trim_samples(row: &[f64], cfg: &AppConfig) -> Vec<f64> {
        let start = row.len().saturating_sub(cfg.width);
        row.iter().skip(start).copied().collect()
    }
}

#[cfg(feature = "audio")]
impl Renderer for AudioRenderer {
    fn mode_name(&self) -> &'static str {
        "audio"
    }

    fn header(&self) -> &'static str {
        "Waveform"
    }

    fn y_bounds(&self, _cfg: &AppConfig) -> (f64, f64) {
        (-1.0, 1.0)
    }

    fn process(&mut self, cfg: &AppConfig, frame: &Matrix) -> Vec<Series> {
        frame
            .iter()
            .take(1)
            .map(|row| Series {
                name: "audio_mix".to_string(),
                color: Color::Cyan,
                samples: Self::trim_samples(row, cfg),
            })
            .collect()
    }
}

#[cfg(feature = "audio")]
fn main() {
    let args = Cli::parse();

    if args.list_devices {
        if let Err(err) = AudioSource::list_input_devices(!args.safe) {
            eprintln!("could not list devices: {err}");
        }
        return;
    }

    if !args.safe {
        eprintln!(
            "system-audio mode active: this will capture playback/monitor sources by default"
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
        !args.safe,
    ) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("audio source init failed: {err}");
            return;
        }
    };

    if let Err(err) = run_app(source, AudioRenderer, config) {
        eprintln!("audio-scope failed: {err:?}");
    }
}

#[cfg(not(feature = "audio"))]
fn main() {
    eprintln!(
        "audio-scope requires the `audio` feature: cargo run --bin audio-scope --features audio"
    );
}
