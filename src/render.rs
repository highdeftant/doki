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

fn render_sonar_bloom(frame: &mut Frame, area: Rect, series: &[Series], background: Color) {
    let width = area.width as usize;
    let height = area.height as usize;
    if width < 3 || height < 3 {
        return;
    }

    let mut cells = vec![vec![None; width]; height];
    let cx = (width as f64 - 1.0) / 2.0;
    let cy = (height as f64 - 1.0) / 2.0;
    let max_radius = cx.min(cy) * 0.9;

    let base = [0.32, 0.56, 0.76];

    for (band_idx, s) in series.iter().take(3).enumerate() {
        if s.samples.is_empty() {
            continue;
        }

        let mut level = 0.0;
        for sample in &s.samples {
            level += sample.abs();
        }
        level = (level / s.samples.len() as f64).clamp(0.0, 1.0);

        let ring_count = 1 + (level * 5.0).round() as usize;
        let base_radius = max_radius * base[band_idx.min(2)] * 0.9;
        let pulse_radius = max_radius * (0.10 + level * 0.45);
        let step = (s.samples.len().max(1) as f64).max(1.0);

        for ring in 0..=ring_count {
            let t = if ring_count == 0 {
                0.0
            } else {
                ring as f64 / ring_count as f64
            };
            let radius = base_radius + t * pulse_radius;
            let points = 72usize.saturating_add((radius * 2.0) as usize);
            let ch = if ring == ring_count {
                '.'
            } else if t < 0.5 {
                '◌'
            } else {
                'o'
            };
            for i in 0..points {
                let angle = (i as f64 / points as f64) * std::f64::consts::TAU;
                let idx = ((angle / std::f64::consts::TAU) * step) as usize % s.samples.len();
                let wobble = s.samples[idx] * (max_radius * 0.08);
                let r = (radius + wobble).max(0.0);
                let x = cx + r * angle.cos();
                let y = cy + r * angle.sin();
                render_grid_point(&mut cells, x, y, s.color, ch);

                if t < 0.33 {
                    let x2 = cx + (r * 0.7) * angle.cos();
                    let y2 = cy + (r * 0.7) * angle.sin();
                    render_grid_point(&mut cells, x2, y2, s.color, '·');
                }
            }
        }
    }

    render_text_grid(frame, area, cells, background);
}

fn render_kaleidoscope(frame: &mut Frame, area: Rect, series: &[Series], background: Color) {
    let width = area.width as usize;
    let height = area.height as usize;
    if width < 3 || height < 3 {
        return;
    }

    let mut cells = vec![vec![None; width]; height];
    let cx = (width as f64 - 1.0) / 2.0;
    let cy = (height as f64 - 1.0) / 2.0;
    let max_radius = cx.min(cy) * 0.9;

    let chars = ['*', '◉', '·'];
    for (band_idx, s) in series.iter().take(3).enumerate() {
        let n = s.samples.len().max(3);
        let mut level = 0.0;
        for sample in &s.samples {
            level += sample.abs();
        }
        let level = (level / s.samples.len().max(1) as f64).clamp(0.0, 1.0);
        let ring_base = max_radius * (0.08 + band_idx as f64 * 0.2);
        for i in 0..n {
            let theta = (i as f64 / n as f64) * (std::f64::consts::FRAC_PI_2);
            let amp = s.samples.get(i).copied().unwrap_or(0.0).abs();
            let radius = ring_base + max_radius * 0.45 * amp + max_radius * 0.10 * level;
            let x = radius * theta.cos();
            let y = radius * theta.sin();

            let points = [
                (x, y),
                (-x, y),
                (x, -y),
                (-x, -y),
                (y, x),
                (-y, x),
                (y, -x),
                (-y, -x),
            ];

            for (px, py) in points {
                let gx = cx + px;
                let gy = cy + py;
                render_grid_point(
                    &mut cells,
                    gx,
                    gy,
                    s.color,
                    chars[band_idx.min(chars.len() - 1)],
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
