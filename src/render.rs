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

fn render_sonar_bloom(frame: &mut Frame, area: Rect, series: &[Series], background: Color) {
    let width = area.width as usize;
    let height = area.height as usize;
    if width < 4 || height < 4 || series.is_empty() {
        return;
    }

    let mut cells = vec![vec![None; width]; height];
    let cx = (width as f64 - 1.0) / 2.0;
    let cy = (height as f64 - 1.0) / 2.0;
    let radius = cx.min(cy) * 0.92;

    let bass = series.get(0);
    let mid = series.get(1);
    let treble = series.get(2);
    let n = bass
        .map(|s| s.samples.len())
        .unwrap_or(0)
        .min(mid.map(|s| s.samples.len()).unwrap_or(0))
        .min(treble.map(|s| s.samples.len()).unwrap_or(0));

    if n < 4 {
        return;
    }

    let pairings: [(usize, usize, Color, char, usize); 3] = [
        (0, 1, series[0].color, '•', 0),
        (1, 2, series[1].color, '◉', 1),
        (2, 0, series[2].color, '◌', 2),
    ];

    let mut prev: [Option<(f64, f64)>; 3] = [None, None, None];
    for i in 0..n {
        let t = i as f64 / (n as f64 - 1.0);
        let spin = t * std::f64::consts::TAU * 2.2;

        for &(a_idx, b_idx, color, ch, pair_idx) in pairings.iter() {
            let a_series = match a_idx {
                0 => bass,
                1 => mid,
                2 => treble,
                _ => None,
            };
            let b_series = match b_idx {
                0 => bass,
                1 => mid,
                2 => treble,
                _ => None,
            };

            let a = if let Some(s) = a_series {
                normalized(&s.samples, sample_at(s, i))
            } else {
                0.0
            };
            let b = if let Some(s) = b_series {
                normalized(&s.samples, sample_at(s, i))
            } else {
                0.0
            };

            let envelope = 0.18 + 0.85 * (0.5 + 0.5 * (spin.sin() * (i as f64 / 8.0).sin())).abs();
            let raw_x = 0.80 * a + 0.30 * b.sin() as f64;
            let raw_y = 0.80 * b + 0.30 * a.cos() as f64;
            let (x_rot, y_rot) = rotate_point(raw_x, raw_y, spin);
            let x = cx + x_rot * radius * envelope;
            let y = cy + y_rot * radius * (envelope * 0.85);

            if pair_idx == 0 {
                render_grid_point(&mut cells, x, y, color, ch);
            }

            if let Some((px, py)) = prev[pair_idx] {
                draw_grid_line(
                    &mut cells,
                    px,
                    py,
                    x,
                    y,
                    color,
                    if i % 2 == 0 { '·' } else { ch },
                );
            }
            prev[pair_idx] = Some((x, y));

            if i % 8 == 0 {
                for glow in 0..2 {
                    let g = glow as f64 * 0.35;
                    let gx =
                        cx + (x_rot * (radius * (envelope + g) * 0.22) + (g * 2.0) * (spin.cos()));
                    let gy =
                        cy + (y_rot * (radius * (envelope + g) * 0.22) + (g * 2.0) * (spin.sin()));
                    render_grid_point(&mut cells, gx, gy, color, '.');
                }
            }
        }
    }

    render_text_grid(frame, area, cells, background);
}

fn render_kaleidoscope(frame: &mut Frame, area: Rect, series: &[Series], background: Color) {
    let width = area.width as usize;
    let height = area.height as usize;
    if width < 4 || height < 4 || series.len() < 3 {
        return;
    }

    let mut cells = vec![vec![None; width]; height];
    let cx = (width as f64 - 1.0) / 2.0;
    let cy = (height as f64 - 1.0) / 2.0;
    let radius = cx.min(cy) * 0.84;

    let bass = series.get(0);
    let mid = series.get(1);
    let treble = series.get(2);
    let n = bass
        .map(|s| s.samples.len())
        .unwrap_or(0)
        .min(mid.map(|s| s.samples.len()).unwrap_or(0))
        .min(treble.map(|s| s.samples.len()).unwrap_or(0));

    if n < 4 {
        return;
    }

    let sectors = 12usize;
    let mut prev: Vec<[Option<(f64, f64)>; 12]> = vec![[None; 12]; 3];

    for i in 0..n {
        let t = i as f64 / (n as f64 - 1.0);
        let bass_s = bass
            .map(|s| normalized(&s.samples, sample_at(s, i)))
            .unwrap_or(0.0);
        let mid_s = mid
            .map(|s| normalized(&s.samples, sample_at(s, i)))
            .unwrap_or(0.0);
        let treble_s = treble
            .map(|s| normalized(&s.samples, sample_at(s, i)))
            .unwrap_or(0.0);

        let pairs = [
            (bass_s, mid_s, series[0].color, '◊', 0usize),
            (mid_s, treble_s, series[1].color, '◆', 1usize),
            (treble_s, bass_s, series[2].color, '✺', 2usize),
        ];

        let twist = t * std::f64::consts::TAU * 1.5;
        for (x_in, y_in, color, ch, pair_idx) in pairs {
            let spin_x = x_in * 0.85 + 0.15 * twist.sin() * y_in;
            let spin_y = y_in * 0.85 + 0.15 * twist.cos() * x_in;

            for sector in 0..sectors {
                let angle = (sector as f64 / sectors as f64) * std::f64::consts::TAU;
                let mut sx = spin_x;
                let sy = spin_y;

                if sector % 2 == 1 {
                    sx = -sx;
                }

                let (rx, ry) = rotate_point(sx, sy, angle);
                let radius_scale =
                    (0.62 + 0.22 * pair_idx as f64 + 0.05 * (i as f64).sin().abs()) * radius;
                let x = cx + rx * radius_scale;
                let y = cy + ry * radius_scale;

                if i == 0 {
                    render_grid_point(&mut cells, x, y, color, ch);
                } else if let Some((px, py)) = prev[pair_idx][sector] {
                    draw_grid_line(
                        &mut cells,
                        px,
                        py,
                        x,
                        y,
                        color,
                        if i % 3 == 0 { '·' } else { ch },
                    );
                }
                prev[pair_idx][sector] = Some((x, y));
            }

            if i % 16 == 0 {
                let spoke_end_x = cx + spin_y.signum() * (radius * 0.35);
                let spoke_end_y = cy + spin_x.signum() * (radius * 0.35);
                let c = if pair_idx == 2 {
                    '|'
                } else if pair_idx == 1 {
                    ':'
                } else {
                    '.'
                };
                draw_grid_line(&mut cells, cx, cy, spoke_end_x, spoke_end_y, color, c);
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
