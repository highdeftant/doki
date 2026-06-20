use ratatui::style::Color;
use std::io;
use std::time::{Duration, Instant};

pub type Matrix = Vec<Vec<f64>>;
pub type SourceFrame = Matrix;

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
            sleep_ms: 16,
        }
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

pub trait Renderer {
    fn mode_name(&self) -> &'static str;
    fn header(&self) -> String;
    fn y_bounds(&self, _cfg: &AppConfig) -> (f64, f64);
    fn process(&mut self, cfg: &AppConfig, frame: &SourceFrame) -> Vec<render::Series>;
    fn handle_event(&mut self, _event: &crossterm::event::Event, _cfg: &mut AppConfig) {}
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
    let mut fps = 0usize;
    let mut framerate = 0usize;
    let mut last_tick = Instant::now();

    loop {
        if let Some(frame) = source.next_frame()? {
            if !cfg.pause {
                let series = renderer.process(&cfg, &frame);
                let (y_min, y_max) = renderer.y_bounds(&cfg);
                render::draw_frame(
                    &mut terminal,
                    &series,
                    &cfg,
                    renderer.mode_name(),
                    &renderer.header(),
                    framerate,
                    y_min,
                    y_max,
                    renderer.background_color(),
                )?;
            }

            fps += 1;
            if last_tick.elapsed().as_secs() >= 1 {
                framerate = fps;
                fps = 0;
                last_tick = Instant::now();
            }
        }

        if crossterm::event::poll(Duration::from_millis(cfg.sleep_ms))? {
            while crossterm::event::poll(Duration::from_millis(0))? {
                let event = crossterm::event::read()?;
                if let crossterm::event::Event::Key(key) = event {
                    if key.kind == crossterm::event::KeyEventKind::Press {
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
                            (_, crossterm::event::KeyCode::Char(' ')) => cfg.pause = !cfg.pause,
                            (_, crossterm::event::KeyCode::Char('h')) => cfg.show_ui = !cfg.show_ui,
                            (_, crossterm::event::KeyCode::Up) => {
                                cfg.scale = (cfg.scale + 0.1).min(8.0)
                            }
                            (_, crossterm::event::KeyCode::Down) => {
                                cfg.scale = (cfg.scale - 0.1).max(0.05)
                            }
                            (_, crossterm::event::KeyCode::Right) => {
                                cfg.zoom = (cfg.zoom + 0.25).min(16.0)
                            }
                            (_, crossterm::event::KeyCode::Left) => {
                                cfg.zoom = (cfg.zoom - 0.25).max(0.5)
                            }
                            (_, _) => {
                                renderer.handle_event(&event, &mut cfg);
                            }
                        }
                    }
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(cfg.sleep_ms));
        }
    }
}

pub mod data;
pub mod render;
