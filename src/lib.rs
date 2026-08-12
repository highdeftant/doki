use ratatui::style::Color;
use std::io;
use std::time::{Duration, Instant};

pub type Matrix = Vec<Vec<f64>>;
pub type SourceFrame = Matrix;

const VIEW_PRESETS: [(f64, f64); 3] = [(1.0, 4.0), (1.5, 2.0), (0.75, 8.0)];

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
            sleep_ms: 42,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VisualStyle {
    Line,
    Persistence,
}

pub trait Renderer {
    fn header(&self) -> String;
    fn y_bounds(&self, _cfg: &AppConfig) -> (f64, f64);
    fn process(&mut self, cfg: &AppConfig, frame: &SourceFrame) -> Vec<render::Series>;
    fn handle_event(&mut self, _event: &crossterm::event::Event, _cfg: &mut AppConfig) {}
    fn visual_style(&self) -> VisualStyle {
        VisualStyle::Line
    }
    fn background_color(&self) -> Color {
        Color::Black
    }
}

pub fn run_app<S: DataSource, R: Renderer>(
    mut source: S,
    mut renderer: R,
    mut cfg: AppConfig,
) -> anyhow::Result<()> {
    let mut terminal = render::init_terminal()?;
    let mut render_scratch = render::RenderScratch::default();

    loop {
        let loop_start = Instant::now();

        if let Some(frame) = source.next_frame()? {
            if !cfg.pause {
                let series = renderer.process(&cfg, &frame);
                let (y_min, y_max) = renderer.y_bounds(&cfg);
                let header = cfg.show_ui.then(|| renderer.header());
                render::draw_frame(
                    &mut terminal,
                    &series,
                    &mut render_scratch,
                    render::RenderParams {
                        cfg: &cfg,
                        header: header.as_deref(),
                        y_bounds: (y_min, y_max),
                        background: renderer.background_color(),
                        style: renderer.visual_style(),
                    },
                )?;
            }
        }

        while crossterm::event::poll(Duration::from_millis(0))? {
            let event = crossterm::event::read()?;
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
                crossterm::event::Event::Mouse(_) => {}
                _ => {}
            }
        }

        if cfg.sleep_ms > 0 {
            if let Some(remaining) =
                Duration::from_millis(cfg.sleep_ms).checked_sub(loop_start.elapsed())
            {
                std::thread::sleep(remaining);
            }
        }
    }
}

pub mod data;
pub mod render;
