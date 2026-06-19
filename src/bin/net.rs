use clap::Parser;
use ratatui::style::Color;
use scope_studio::{
    data::network::NetSource, render::Series, run_app, AppConfig, Matrix, Renderer,
};

#[derive(Parser)]
#[command(name = "net-scope")]
struct Cli {
    /// Wireless interface to sample from (auto-detect)
    #[arg(short, long)]
    interface: Option<String>,
    /// Width of plotted history in samples
    #[arg(short = 'w', long, default_value_t = 240)]
    width: usize,
    /// Polling sleep in ms
    #[arg(short = 's', long, default_value_t = 250)]
    sleep_ms: u64,
    /// How many samples to keep in memory
    #[arg(short = 'n', long, default_value_t = 120)]
    history: usize,
}

#[derive(Default)]
struct NetRenderer {
    last_minmax: (f64, f64),
}

impl NetRenderer {
    fn trim_samples(row: &[f64], cfg: &AppConfig) -> Vec<f64> {
        let start = row.len().saturating_sub(cfg.width);
        row.iter().skip(start).copied().collect()
    }

    fn calculate_bounds(frame: &Matrix) -> (f64, f64) {
        let mut min_v = f64::INFINITY;
        let mut max_v = f64::NEG_INFINITY;
        for row in frame {
            for &v in row {
                min_v = min_v.min(v);
                max_v = max_v.max(v);
            }
        }

        if !min_v.is_finite() || !max_v.is_finite() || (max_v - min_v).abs() < f64::EPSILON {
            return (-100.0, 100.0);
        }
        let pad = (max_v - min_v) * 0.1;
        (min_v - pad, max_v + pad)
    }
}

impl Renderer for NetRenderer {
    fn mode_name(&self) -> &'static str {
        "net"
    }

    fn header(&self) -> &'static str {
        "RSSI / Throughput"
    }

    fn y_bounds(&self, _cfg: &AppConfig) -> (f64, f64) {
        self.last_minmax
    }

    fn process(&mut self, cfg: &AppConfig, frame: &Matrix) -> Vec<Series> {
        self.last_minmax = Self::calculate_bounds(frame);

        let names = ["rssi_dbm", "noise_dbm", "rx_mbps", "tx_mbps"];
        let colors = [Color::Cyan, Color::Blue, Color::Magenta, Color::Green];

        frame
            .iter()
            .enumerate()
            .map(|(idx, row)| Series {
                name: names.get(idx).copied().unwrap_or("signal").to_string(),
                color: *colors.get(idx).unwrap_or(&Color::White),
                samples: Self::trim_samples(row, cfg),
            })
            .collect()
    }
}

fn main() {
    let args = Cli::parse();

    let source =
        NetSource::new(args.interface, args.history).expect("failed to initialize network source");

    let config = AppConfig {
        width: args.width,
        sleep_ms: args.sleep_ms,
        ..Default::default()
    };

    let renderer = NetRenderer::default();
    if let Err(err) = run_app(source, renderer, config) {
        eprintln!("net-scope failed: {err:?}");
    }
}
