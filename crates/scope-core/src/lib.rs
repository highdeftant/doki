use ratatui::style::Color;
use std::io;
use std::time::{Duration, Instant};

pub type Matrix = Vec<Vec<f64>>;
pub type SourceFrame = Matrix;

const VIEW_PRESETS: [(f64, f64); 3] = [(1.0, 4.0), (1.5, 2.0), (0.75, 8.0)];
const LISSAJOUS_ROTATION_PERIOD: Duration = Duration::from_secs(60);

fn lissajous_rotation(elapsed: Duration) -> f64 {
    let period = LISSAJOUS_ROTATION_PERIOD.as_secs_f64();
    std::f64::consts::TAU * (elapsed.as_secs_f64() % period) / period
}

pub trait DataSource {
    fn next_frame(&mut self) -> io::Result<Option<SourceFrame>>;
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub pause: bool,
    pub width: usize,
    pub zoom: f64,
    pub scale: f64,
    pub show_ui: bool,
    pub show_help: bool,
    pub view_preset: usize,
    pub sleep_ms: u64,
    /// Physical text-cell width / height; half-width cells are the fallback.
    pub cell_aspect: f64,
    /// Display-only stereo plot rotation, in radians.
    pub lissajous_rotation: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            pause: false,
            width: 512,
            zoom: 4.0,
            scale: 1.0,
            show_ui: true,
            show_help: false,
            view_preset: 0,
            sleep_ms: 16,
            cell_aspect: 0.5,
            lissajous_rotation: 0.0,
        }
    }
}

impl AppConfig {
    pub fn reset_view(&mut self) {
        self.apply_view_preset(0);
    }

    pub fn cycle_view_preset(&mut self) {
        self.apply_view_preset((self.view_preset + 1) % VIEW_PRESETS.len());
    }

    fn apply_view_preset(&mut self, preset: usize) {
        self.view_preset = preset % VIEW_PRESETS.len();
        let (scale, zoom) = VIEW_PRESETS[self.view_preset];
        self.scale = scale;
        self.zoom = zoom;
    }
}

#[derive(Debug)]
pub struct RingBuffer {
    pub data: std::collections::VecDeque<f64>,
    pub cap: usize,
}

impl RingBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            data: std::collections::VecDeque::with_capacity(cap),
            cap,
        }
    }

    pub fn push(&mut self, value: f64) {
        if self.data.len() == self.cap {
            let _ = self.data.pop_front();
        }
        self.data.push_back(value);
    }

    pub fn snapshot(&self) -> Vec<f64> {
        self.data.iter().copied().collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum VisualStyle {
    Line,
    OceanMist,
    Bars,
    Spectrogram,
    Lissajous,
    Persistence,
}

pub trait Renderer {
    fn header(&self) -> String;
    fn y_bounds(&self, _cfg: &AppConfig) -> (f64, f64);
    fn process(&mut self, cfg: &AppConfig, frame: &SourceFrame, series: &mut Vec<render::Series>);
    fn handle_event(&mut self, _event: &crossterm::event::Event, _cfg: &mut AppConfig) {}
    fn visual_style(&self) -> VisualStyle {
        VisualStyle::Line
    }
    fn background_color(&self) -> Color {
        Color::Black
    }
}

struct FramePacer {
    period: Duration,
    next_frame: Duration,
}

impl FramePacer {
    fn new(sleep_ms: u64) -> Self {
        Self {
            period: Duration::from_millis(sleep_ms.saturating_mul(2).max(1)),
            next_frame: Duration::ZERO,
        }
    }

    fn claim_frame(&mut self, now: Duration) -> bool {
        if now < self.next_frame {
            return false;
        }
        // Schedule from this frame, not an overdue deadline, to avoid catch-up bursts.
        self.next_frame = now.saturating_add(self.period);
        true
    }

    fn time_until_frame(&self, now: Duration) -> Duration {
        self.next_frame.saturating_sub(now)
    }
}

fn cell_aspect(size: crossterm::terminal::WindowSize) -> f64 {
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return 0.5;
    }
    f64::from(size.width) * f64::from(size.rows)
        / (f64::from(size.height) * f64::from(size.columns))
}

fn terminal_cell_aspect() -> f64 {
    crossterm::terminal::window_size()
        .map(cell_aspect)
        .unwrap_or(0.5)
}

fn event_invalidates_frame(event: &crossterm::event::Event) -> bool {
    match event {
        crossterm::event::Event::Resize(_, _) => true,
        crossterm::event::Event::Key(key) => key.kind == crossterm::event::KeyEventKind::Press,
        _ => false,
    }
}

pub fn run_app<S: DataSource, R: Renderer>(
    mut source: S,
    mut renderer: R,
    mut cfg: AppConfig,
) -> anyhow::Result<()> {
    let mut terminal = render::init_terminal()?;
    cfg.cell_aspect = terminal_cell_aspect();

    let mut series = Vec::new();
    let mut pacer = FramePacer::new(cfg.sleep_ms);
    let mut redraw_pending = true;
    let epoch = Instant::now();
    loop {
        if pacer.claim_frame(epoch.elapsed()) {
            if let Some(frame) = source.next_frame()? {
                if !cfg.pause {
                    renderer.process(&cfg, &frame, &mut series);
                    redraw_pending = true;
                }
            }
            if matches!(renderer.visual_style(), VisualStyle::Lissajous) {
                cfg.lissajous_rotation = lissajous_rotation(epoch.elapsed());
                redraw_pending = true;
            }
        }

        if redraw_pending {
            let (y_min, y_max) = renderer.y_bounds(&cfg);
            render::draw_frame(
                &mut terminal,
                &series,
                &cfg,
                &renderer.header(),
                (y_min, y_max),
                renderer.background_color(),
                renderer.visual_style(),
            )?;
            redraw_pending = false;
        }

        if crossterm::event::poll(pacer.time_until_frame(epoch.elapsed()))? {
            let event = crossterm::event::read()?;
            let redraw_after_event = event_invalidates_frame(&event);
            match event {
                crossterm::event::Event::Key(key)
                    if key.kind == crossterm::event::KeyEventKind::Press =>
                {
                    match (key.modifiers, key.code) {
                        (
                            crossterm::event::KeyModifiers::CONTROL,
                            crossterm::event::KeyCode::Char('c'),
                        )
                        | (
                            crossterm::event::KeyModifiers::CONTROL,
                            crossterm::event::KeyCode::Char('q'),
                        ) => {
                            render::restore_terminal()?;
                            return Ok(());
                        }
                        (_, crossterm::event::KeyCode::Char('q')) => {
                            render::restore_terminal()?;
                            return Ok(());
                        }
                        (_, crossterm::event::KeyCode::Esc)
                        | (_, crossterm::event::KeyCode::Char('?'))
                            if cfg.show_help =>
                        {
                            cfg.show_help = false;
                        }
                        (_, _) if cfg.show_help => {}
                        (_, crossterm::event::KeyCode::Char('?')) => cfg.show_help = true,
                        (_, crossterm::event::KeyCode::Char('r')) => cfg.reset_view(),
                        (_, crossterm::event::KeyCode::Char('p')) => cfg.cycle_view_preset(),
                        (_, crossterm::event::KeyCode::Char(' ')) => cfg.pause = !cfg.pause,
                        (_, crossterm::event::KeyCode::Char('h')) => cfg.show_ui = !cfg.show_ui,
                        (_, crossterm::event::KeyCode::Up) => {
                            cfg.scale = (cfg.scale + 0.1).min(8.0);
                        }
                        (_, crossterm::event::KeyCode::Down) => {
                            cfg.scale = (cfg.scale - 0.1).max(0.05);
                        }
                        (_, crossterm::event::KeyCode::Right) => {
                            cfg.zoom = (cfg.zoom + 0.25).min(16.0);
                        }
                        (_, crossterm::event::KeyCode::Left) => {
                            cfg.zoom = (cfg.zoom - 0.25).max(0.5);
                        }
                        (_, _) => {
                            renderer.handle_event(&event, &mut cfg);
                        }
                    }
                }
                crossterm::event::Event::Resize(_, _) => {
                    cfg.cell_aspect = terminal_cell_aspect();
                }
                crossterm::event::Event::Mouse(_) => {}
                _ => {}
            }
            if redraw_after_event {
                redraw_pending = true;
            }
        }
    }
}

pub mod render;
pub use render::Series;

#[cfg(test)]
mod tests {
    use super::{cell_aspect, event_invalidates_frame, lissajous_rotation, AppConfig, FramePacer};
    use std::time::Duration;

    #[test]
    fn resize_event_invalidates_frame_without_waiting_for_new_audio() {
        let event = crossterm::event::Event::Resize(80, 24);
        assert!(event_invalidates_frame(&event));

        let key = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('h'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(event_invalidates_frame(&key));
    }

    #[test]
    fn terminal_pixels_calibrate_cell_aspect_and_missing_metrics_use_half_width() {
        use crossterm::terminal::WindowSize;
        assert_eq!(
            cell_aspect(WindowSize {
                columns: 120,
                rows: 30,
                width: 1200,
                height: 600
            }),
            0.5
        );
        assert_eq!(
            cell_aspect(WindowSize {
                columns: 120,
                rows: 30,
                width: 1800,
                height: 600
            }),
            0.75
        );
        for (width, height, columns, rows) in [
            (0, 600, 120, 30),
            (1200, 0, 120, 30),
            (1200, 600, 0, 30),
            (1200, 600, 120, 0),
        ] {
            assert_eq!(
                cell_aspect(WindowSize {
                    columns,
                    rows,
                    width,
                    height
                }),
                0.5
            );
        }
    }

    #[test]
    fn frame_pacer_keeps_deadline_during_mouse_and_key_arrivals() {
        let mut pacer = FramePacer::new(AppConfig::default().sleep_ms);
        assert_eq!(pacer.period, Duration::from_millis(32));
        assert_eq!(pacer.time_until_frame(Duration::ZERO), Duration::ZERO);
        assert!(pacer.claim_frame(Duration::ZERO));

        let deadline = Duration::from_millis(32);
        for (input, arrival_ms) in [
            ("mouse", 1),
            ("key", 2),
            ("mouse", 8),
            ("key", 16),
            ("mouse", 24),
            ("key", 31),
            ("mouse", 31),
        ] {
            let now = Duration::from_millis(arrival_ms);
            assert!(!pacer.claim_frame(now), "{input} at {arrival_ms} ms");
            assert_eq!(pacer.next_frame, deadline);
            assert_eq!(pacer.time_until_frame(now), deadline - now);
            assert_eq!(pacer.next_frame, deadline);
        }

        assert_eq!(pacer.time_until_frame(deadline), Duration::ZERO);
        assert!(pacer.claim_frame(deadline));
        assert_eq!(pacer.next_frame, Duration::from_millis(64));
        assert_eq!(pacer.time_until_frame(deadline), Duration::from_millis(32));
        assert!(!pacer.claim_frame(Duration::from_millis(63)));
        assert!(pacer.claim_frame(Duration::from_millis(64)));
    }

    #[test]
    fn lissajous_rotation_is_time_based_and_wraps_once_per_minute() {
        assert_eq!(lissajous_rotation(Duration::ZERO), 0.0);
        assert!(
            (lissajous_rotation(Duration::from_secs(15)) - std::f64::consts::FRAC_PI_2).abs()
                < 1e-12
        );
        assert!((lissajous_rotation(Duration::from_secs(30)) - std::f64::consts::PI).abs() < 1e-12);
        assert!(
            (lissajous_rotation(Duration::from_secs(45)) - 3.0 * std::f64::consts::FRAC_PI_2).abs()
                < 1e-12
        );
        assert_eq!(lissajous_rotation(Duration::from_secs(60)), 0.0);
        assert_eq!(
            lissajous_rotation(Duration::from_secs(75)),
            lissajous_rotation(Duration::from_secs(15))
        );
    }

    #[test]
    fn frame_pacer_skips_missed_deadlines_without_catch_up() {
        let mut pacer = FramePacer::new(16);
        assert!(pacer.claim_frame(Duration::ZERO));
        let overdue = Duration::from_millis(100);
        assert_eq!(pacer.time_until_frame(overdue), Duration::ZERO);
        assert_eq!(pacer.next_frame, Duration::from_millis(32));
        assert!(pacer.claim_frame(overdue));
        assert_eq!(pacer.next_frame, Duration::from_millis(132));

        for elapsed_ms in [100, 101, 112, 131] {
            let now = Duration::from_millis(elapsed_ms);
            assert!(!pacer.claim_frame(now));
            assert_eq!(pacer.next_frame, Duration::from_millis(132));
            assert_eq!(
                pacer.time_until_frame(now),
                Duration::from_millis(132 - elapsed_ms)
            );
        }
        assert!(pacer.claim_frame(Duration::from_millis(132)));
        assert_eq!(pacer.next_frame, Duration::from_millis(164));
    }

    #[test]
    fn frame_pacer_uses_one_millisecond_minimum_period() {
        let mut pacer = FramePacer::new(0);
        assert_eq!(pacer.period, Duration::from_millis(1));
        assert!(pacer.claim_frame(Duration::ZERO));
        assert!(!pacer.claim_frame(Duration::ZERO));
        assert_eq!(
            pacer.time_until_frame(Duration::ZERO),
            Duration::from_millis(1)
        );
        assert!(pacer.claim_frame(Duration::from_millis(1)));
        assert_eq!(pacer.next_frame, Duration::from_millis(2));
    }

    #[test]
    fn frame_pacer_saturates_period_and_deadline() {
        let mut pacer = FramePacer::new(u64::MAX);
        assert_eq!(pacer.period, Duration::from_millis(u64::MAX));
        let now = Duration::MAX - Duration::from_millis(1);
        assert!(pacer.claim_frame(now));
        assert_eq!(pacer.next_frame, Duration::MAX);
        assert_eq!(pacer.time_until_frame(now), Duration::from_millis(1));
        assert!(!pacer.claim_frame(now));
    }
}
