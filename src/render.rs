use std::io;
use std::io::stdout;

use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Block, Chart, Dataset, GraphType, Paragraph},
    Frame, Terminal,
};

use crate::VisualStyle;

#[derive(Clone)]
pub struct Series {
    pub name: String,
    pub color: Color,
    pub samples: Vec<f64>,
}

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    color: Color,
}

pub fn init_terminal() -> io::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    Ok(terminal)
}

pub fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)
}

pub fn draw_frame(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    series: &[Series],
    cfg: &crate::AppConfig,
    _mode_name: &str,
    header: &str,
    _fps: usize,
    y_min: f64,
    y_max: f64,
    background: Color,
    style: VisualStyle,
) -> io::Result<()> {
    terminal.draw(|frame| {
        if cfg.show_ui {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)].as_ref())
                .split(frame.area());

            let head_style = Style::default().fg(Color::White).bg(background);
            let head_text = header;
            let header = Paragraph::new(Line::from(head_text)).style(head_style);
            frame.render_widget(header, layout[0]);
            render_chart(
                frame, layout[1], series, cfg, y_min, y_max, background, style,
            );
        } else {
            render_chart(
                frame,
                frame.area(),
                series,
                cfg,
                y_min,
                y_max,
                background,
                style,
            );
        }
    })?;

    Ok(())
}

fn render_chart(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    cfg: &crate::AppConfig,
    y_min: f64,
    y_max: f64,
    background: Color,
    style: VisualStyle,
) {
    match style {
        VisualStyle::Line => render_line_chart(frame, area, series, cfg, y_min, y_max, background),
        VisualStyle::Sonar => render_sonar_bloom(frame, area, series, background),
        VisualStyle::Kaleidoscope => render_kaleidoscope(frame, area, series, background),
    }
}

fn render_line_chart(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    cfg: &crate::AppConfig,
    y_min: f64,
    y_max: f64,
    background: Color,
) {
    let target_cols = area.width.max(2) as usize;
    let visible_len = ((target_cols as f64) * cfg.zoom)
        .round()
        .clamp(2.0, cfg.width.max(2) as f64) as usize;

    let point_sets: Vec<Vec<(f64, f64)>> = series
        .iter()
        .map(|s| resample_samples(&s.samples, visible_len, target_cols))
        .collect();

    let datasets: Vec<Dataset> = series
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            let points = &point_sets[idx];
            Dataset::default()
                .name(s.name.clone())
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(s.color))
                .data(points)
        })
        .collect();

    let padded_max = if y_max > y_min {
        y_max + 0.001
    } else {
        y_min + 1.0
    };

    let block = Block::default().style(Style::default().bg(background));
    let chart = Chart::new(datasets)
        .block(block)
        .style(Style::default().bg(background))
        .x_axis(
            Axis::default()
                .bounds([0.0, (target_cols.saturating_sub(1)) as f64])
                .labels(Vec::<Span>::new())
                .style(Style::default().fg(Color::DarkGray)),
        )
        .y_axis(
            Axis::default()
                .bounds([y_min * cfg.scale, padded_max * cfg.scale])
                .style(Style::default().fg(Color::DarkGray)),
        );

    frame.render_widget(chart, area);
}

fn render_grid_point(cells: &mut [Vec<Option<Cell>>], x: f64, y: f64, color: Color, ch: char) {
    let row = y.round() as isize;
    let col = x.round() as isize;
    if row < 0 || col < 0 {
        return;
    }
    let row = row as usize;
    let col = col as usize;

    if row >= cells.len() {
        return;
    }
    if col >= cells[row].len() {
        return;
    }

    cells[row][col] = Some(Cell { ch, color });
}

fn render_text_grid(
    frame: &mut Frame,
    area: Rect,
    cells: Vec<Vec<Option<Cell>>>,
    background: Color,
) {
    let h = cells.len();
    let mut lines = Vec::with_capacity(h);
    for row in cells {
        let mut spans = Vec::with_capacity(row.len());
        for cell in row {
            match cell {
                Some(Cell { ch, color }) => spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(color).bg(background),
                )),
                None => spans.push(Span::styled(" ", Style::default().bg(background))),
            }
        }
        lines.push(Line::from(spans));
    }

    let block = Block::default().style(Style::default().bg(background));
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(background))
            .block(block),
        area,
    );
}

fn sample_at(series: &Series, idx: usize) -> f64 {
    series.samples.get(idx).copied().unwrap_or(0.0)
}

fn max_abs(samples: &[f64]) -> f64 {
    samples.iter().map(|s| s.abs()).fold(0.0f64, f64::max)
}

fn normalized(samples: &[f64], sample: f64) -> f64 {
    let peak = max_abs(samples);
    if peak <= f64::EPSILON {
        0.0
    } else {
        (sample / peak).clamp(-1.0, 1.0)
    }
}

fn rotate_point(x: f64, y: f64, angle: f64) -> (f64, f64) {
    let (sa, ca) = angle.sin_cos();
    (x * ca - y * sa, x * sa + y * ca)
}

fn draw_grid_line(
    cells: &mut [Vec<Option<Cell>>],
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    color: Color,
    ch: char,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let steps = (dx.abs().max(dy.abs()).ceil() as usize).max(1);
    let inv = 1.0 / steps as f64;

    for step in 0..=steps {
        let t = step as f64 * inv;
        let x = x0 + dx * t;
        let y = y0 + dy * t;
        render_grid_point(cells, x, y, color, ch);
    }
}

/// Terminal character cells are roughly twice as tall as they are wide.
/// Multiply the y-displacement by this factor to get a visually round circle,
/// otherwise a parametric circle will render as a vertically-stretched ellipse.
const TERMINAL_CELL_ASPECT: f64 = 0.5;

/// Number of parametric steps used to trace a full revolution. Chosen so
/// consecutive points are ~1 cell apart even for large radii, keeping the
/// outline smooth without overshooting the rasterizer.
const CIRCLE_STEPS: usize = 256;

/// Render a geometrically perfect circle at (cx, cy) with radius `radius`,
/// using parametric equations:
///   x(θ) = cx + radius * cos(θ)
///   y(θ) = cy + radius * sin(θ) * TERMINAL_CELL_ASPECT
fn draw_circle_cells(
    cells: &mut [Vec<Option<Cell>>],
    cx: f64,
    cy: f64,
    radius: f64,
    color: Color,
    ch: char,
) {
    let mut prev: Option<(f64, f64)> = None;
    for i in 0..=CIRCLE_STEPS {
        let theta = (i as f64 / CIRCLE_STEPS as f64) * std::f64::consts::TAU;
        let x = cx + radius * theta.cos();
        let y = cy + radius * theta.sin() * TERMINAL_CELL_ASPECT;
        render_grid_point(cells, x, y, color, ch);
        if let Some((px, py)) = prev {
            draw_grid_line(cells, px, py, x, y, color, ch);
        }
        prev = Some((x, y));
    }
}

fn render_sonar_bloom(frame: &mut Frame, area: Rect, series: &[Series], background: Color) {
    let width = area.width as usize;
    let height = area.height as usize;
    if width < 6 || height < 6 || series.is_empty() {
        return;
    }

    let mut cells = vec![vec![None; width]; height];
    let cx = (width as f64 - 1.0) / 2.0;
    let cy = (height as f64 - 1.0) / 2.0;

    // With y scaled by TERMINAL_CELL_ASPECT (=0.5) we must pick the max radius
    // from the tighter of (horizontal / aspect) and (vertical). Otherwise the
    // circle bleeds off whichever axis is narrower.
    let max_radius = (cx / 1.0).min(cy / TERMINAL_CELL_ASPECT) * 0.90;

    // Three per-band circles: bass (inner), mid, treble (outer).
    // Audio modulation is intentionally *not* applied here yet — this is the
    // clean geometric baseline. Amplitude layering comes next.
    let band_specs: [(usize, char, f64); 3] = [(0, '◌', 0.35), (1, '◉', 0.60), (2, '●', 0.85)];

    for (band_idx, glyph, radius_factor) in band_specs {
        let color = series
            .get(band_idx)
            .map(|s| s.color)
            .unwrap_or(Color::White);
        draw_circle_cells(&mut cells, cx, cy, max_radius * radius_factor, color, glyph);
    }

    render_text_grid(frame, area, cells, background);
}

fn render_kaleidoscope(frame: &mut Frame, area: Rect, series: &[Series], background: Color) {
    let width = area.width as usize;
    let height = area.height as usize;
    if width < 8 || height < 8 || series.len() < 3 {
        return;
    }

    let mut cells = vec![vec![None; width]; height];
    let cx = (width as f64 - 1.0) / 2.0;
    let cy = (height as f64 - 1.0) / 2.0;
    let max_radius = cx.min(cy) * 0.88;

    let sectors: usize = 10;
    let wraps = 3.6_f64;
    let band_chars = ['◊', '◆', '✺'];

    for band_idx in 0..series.len().min(3) {
        let series = &series[band_idx];
        let n = series.samples.len();
        if n < 12 {
            continue;
        }

        let mut prev = vec![None; sectors];
        let ring_bias = 0.22 + band_idx as f64 * 0.24;
        let phase = band_idx as f64 * 0.65;

        for sample_idx in 0..n {
            let t = sample_idx as f64 / (n as f64 - 1.0);
            let wave = normalized(&series.samples, sample_at(series, sample_idx));
            let amplitude = wave.abs().clamp(0.0, 1.0);
            let radius =
                max_radius * (ring_bias + 0.35 * amplitude + 0.12 * (t * 24.0).sin().abs());
            let base_angle =
                t * std::f64::consts::TAU * wraps + phase + (sample_idx as f64 / 16.0).sin() * 0.3;

            for sector in 0..sectors {
                let arm_angle = (sector as f64 / sectors as f64) * std::f64::consts::TAU;
                let px = radius * base_angle.cos();
                let py = radius * base_angle.sin() * (1.0 - 0.35 * wave.abs());

                let (sx, sy) = if sector % 2 == 1 { (-px, py) } else { (px, py) };

                let (rx, ry) = rotate_point(sx, sy, arm_angle);
                let x = cx + rx;
                let y = cy + ry;

                if sample_idx == 0 {
                    render_grid_point(&mut cells, x, y, series.color, band_chars[band_idx.min(2)]);
                } else if let Some((tx, ty)) = prev[sector] {
                    draw_grid_line(
                        &mut cells,
                        tx,
                        ty,
                        x,
                        y,
                        series.color,
                        if sample_idx % 3 == 0 {
                            if band_idx == 2 {
                                '·'
                            } else {
                                band_chars[band_idx.min(2)]
                            }
                        } else {
                            band_chars[band_idx.min(2)]
                        },
                    );
                }

                prev[sector] = Some((x, y));
            }

            if sample_idx % 18 == 0 {
                let spoke = std::f64::consts::TAU * (phase + t);
                let spoke_end_x = cx + max_radius * (0.32 + 0.06 * band_idx as f64) * spoke.cos();
                let spoke_end_y = cy + max_radius * (0.32 + 0.06 * band_idx as f64) * spoke.sin();
                draw_grid_line(
                    &mut cells,
                    cx,
                    cy,
                    spoke_end_x,
                    spoke_end_y,
                    series.color,
                    if band_idx == 2 { '|' } else { ':' },
                );
            }
        }
    }

    render_text_grid(frame, area, cells, background);
}

fn resample_samples(samples: &[f64], visible_len: usize, target_cols: usize) -> Vec<(f64, f64)> {
    let target_cols = target_cols.max(2);
    let visible_len = visible_len.max(2);
    let start = samples.len().saturating_sub(visible_len);
    let window = &samples[start..];

    if window.is_empty() {
        return vec![(0.0, 0.0); target_cols];
    }

    if window.len() == 1 {
        return (0..target_cols).map(|x| (x as f64, window[0])).collect();
    }

    let last_src = (window.len() - 1) as f64;
    let denom = (target_cols - 1) as f64;

    (0..target_cols)
        .map(|col| {
            let pos = (col as f64) * last_src / denom;
            let lo = pos.floor() as usize;
            let hi = pos.ceil() as usize;
            let frac = pos - lo as f64;
            let y = if lo == hi {
                window[lo]
            } else {
                window[lo] * (1.0 - frac) + window[hi] * frac
            };
            (col as f64, y)
        })
        .collect()
}
