use std::io;
use std::io::stdout;

use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Clear, Dataset, GraphType, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::VisualStyle;

#[derive(Clone)]
pub struct Series {
    pub name: String,
    pub color: Color,
    pub samples: Vec<f64>,
}

#[derive(Default)]
pub struct RenderScratch {
    point_sets: Vec<Vec<(f64, f64)>>,
}

#[derive(Clone, Copy)]
pub struct RenderParams<'a> {
    pub cfg: &'a crate::AppConfig,
    pub header: Option<&'a str>,
    pub y_bounds: (f64, f64),
    pub background: Color,
    pub style: VisualStyle,
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
    scratch: &mut RenderScratch,
    params: RenderParams<'_>,
) -> io::Result<()> {
    terminal.draw(|frame| {
        if params.cfg.show_ui {
            if let Some(header_text) = params.header {
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0)].as_ref())
                    .split(frame.area());

                let head_style = Style::default().fg(Color::White).bg(params.background);
                let header = Paragraph::new(Line::from(header_text)).style(head_style);
                frame.render_widget(header, layout[0]);
                render_chart(frame, layout[1], series, scratch, params);
            } else {
                render_chart(frame, frame.area(), series, scratch, params);
            }
        } else {
            render_chart(frame, frame.area(), series, scratch, params);
        }
        if params.cfg.show_help {
            render_help_popup(frame, params.background);
        }
    })?;

    Ok(())
}

fn render_help_popup(frame: &mut Frame, background: Color) {
    let area = centered_rect(frame.area(), 54, 16);
    let text = vec![
        Line::from("keyboard controls"),
        Line::from(""),
        Line::from("q / Ctrl-C / Ctrl-Q  quit"),
        Line::from("Space              pause / resume"),
        Line::from("h                  hide / show header"),
        Line::from("r                  reset view"),
        Line::from("p                  cycle view preset"),
        Line::from("w                  cycle waves: 3 / 2 / 1"),
        Line::from("↑ / ↓              gain up / down"),
        Line::from("← / →              zoom out / in"),
        Line::from("t                  cycle theme"),
        Line::from("v                  cycle visual mode: line / persistence"),
        Line::from("b                  cycle background"),
        Line::from("c                  toggle rgb wave cycling"),
        Line::from("Esc / ?            close this window"),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White).bg(background));
    let popup = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Left)
        .style(Style::default().fg(Color::White).bg(background))
        .wrap(Wrap { trim: true });

    frame.render_widget(Clear, area);
    frame.render_widget(popup, area);
}

fn centered_rect(parent: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(parent.width);
    let height = height.min(parent.height);
    let x = parent.x + parent.width.saturating_sub(width) / 2;
    let y = parent.y + parent.height.saturating_sub(height) / 2;

    Rect {
        x,
        y,
        width,
        height,
    }
}

fn render_chart(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    scratch: &mut RenderScratch,
    params: RenderParams<'_>,
) {
    match params.style {
        VisualStyle::Line | VisualStyle::Persistence => {
            render_line_chart(frame, area, series, scratch, params)
        }
    }
}

fn render_line_chart(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    scratch: &mut RenderScratch,
    params: RenderParams<'_>,
) {
    let target_cols = area.width.max(2) as usize;
    let visible_len = ((target_cols as f64) * params.cfg.zoom)
        .round()
        .clamp(2.0, params.cfg.width.max(2) as f64) as usize;
    let (y_min, y_max) = params.y_bounds;

    scratch.point_sets.truncate(series.len());
    if scratch.point_sets.len() < series.len() {
        scratch
            .point_sets
            .resize_with(series.len(), Vec::<(f64, f64)>::new);
    }

    for (idx, s) in series.iter().enumerate() {
        let points = &mut scratch.point_sets[idx];
        resample_samples_into(&s.samples, visible_len, target_cols, points);
    }

    let mut datasets = Vec::with_capacity(series.len());
    for (s, points) in series.iter().zip(scratch.point_sets.iter()) {
        datasets.push(
            Dataset::default()
                .name(s.name.as_str())
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(s.color))
                .data(points.as_slice()),
        );
    }

    let padded_max = if y_max > y_min {
        y_max + 0.001
    } else {
        y_min + 1.0
    };

    let block = Block::default().style(Style::default().bg(params.background));
    let chart = Chart::new(datasets)
        .block(block)
        .style(Style::default().bg(params.background))
        .x_axis(
            Axis::default()
                .bounds([0.0, (target_cols.saturating_sub(1)) as f64])
                .labels(Vec::<Span>::new())
                .style(Style::default().fg(Color::DarkGray)),
        )
        .y_axis(
            Axis::default()
                .bounds([y_min * params.cfg.scale, padded_max * params.cfg.scale])
                .style(Style::default().fg(Color::DarkGray)),
        );

    frame.render_widget(chart, area);
}

fn resample_samples_into(
    samples: &[f64],
    visible_len: usize,
    target_cols: usize,
    out: &mut Vec<(f64, f64)>,
) {
    let target_cols = target_cols.max(2);
    let visible_len = visible_len.max(2);
    out.clear();
    out.reserve(target_cols);

    let start = samples.len().saturating_sub(visible_len);
    let window = &samples[start..];

    if window.is_empty() {
        out.extend((0..target_cols).map(|x| (x as f64, 0.0)));
        return;
    }

    if window.len() == 1 {
        out.extend((0..target_cols).map(|x| (x as f64, window[0])));
        return;
    }

    let last_src = (window.len() - 1) as f64;
    let denom = (target_cols - 1) as f64;

    out.extend((0..target_cols).map(|col| {
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
    }));
}
