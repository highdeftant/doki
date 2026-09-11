use std::io;
use std::io::stdout;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
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

pub fn init_terminal() -> io::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    Ok(terminal)
}

pub fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen)
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
        if cfg.show_help {
            render_help_popup(frame, background);
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
        Line::from("b                  cycle background"),
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
    cfg: &crate::AppConfig,
    y_min: f64,
    y_max: f64,
    background: Color,
    style: VisualStyle,
) {
    match style {
        VisualStyle::Line => render_line_chart(frame, area, series, cfg, y_min, y_max, background),
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
