use std::io;
use std::io::stdout;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::canvas::{Canvas, Line as CanvasLine, Painter, Points},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::VisualStyle;

const SPECTRUM_PEAK_TARGET: f64 = 0.65;
const SPECTRUM_REFERENCE_FLOOR: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    Original,
    Rgb,
    Mono,
}

/// Saturated global hue: one continuous revolution per 45 seconds.
pub fn rgb_at_time(elapsed: std::time::Duration) -> Color {
    const CYCLE_SECONDS: f64 = 45.0;
    let hue = elapsed.as_secs_f64().rem_euclid(CYCLE_SECONDS) / (CYCLE_SECONDS / 6.0);
    let peak = 255.0;
    let secondary = peak * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match hue as u8 {
        0 => (peak, secondary, 0.0),
        1 => (secondary, peak, 0.0),
        2 => (0.0, peak, secondary),
        3 => (0.0, secondary, peak),
        4 => (secondary, 0.0, peak),
        _ => (peak, 0.0, secondary),
    };
    Color::Rgb(red.round() as u8, green.round() as u8, blue.round() as u8)
}

/// Change intensity without introducing another hue.
pub fn shade_color(color: Color, brightness: f64) -> Color {
    match color {
        Color::Rgb(r, g, b) => {
            let brightness = brightness.clamp(0.0, 1.0);
            Color::Rgb(
                (f64::from(r) * brightness).round() as u8,
                (f64::from(g) * brightness).round() as u8,
                (f64::from(b) * brightness).round() as u8,
            )
        }
        _ => color,
    }
}

#[derive(Clone)]
pub struct Series {
    pub name: String,
    pub color: Color,
    pub palette: Palette,
    pub samples: Vec<f64>,
    pub peak_levels: Vec<f64>,
    pub max_frequency: f64,
    /// Synchronized raw (left, right) samples; scalar views leave this empty.
    pub xy_points: Vec<(f64, f64)>,
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

fn wrapped_line_count(text: &str, width: u16) -> usize {
    let width = usize::from(width.max(1));
    text.lines()
        .map(|line| line.chars().count().max(1).div_ceil(width))
        .sum::<usize>()
        .max(1)
}

pub fn draw_frame<B: Backend>(
    terminal: &mut Terminal<B>,
    series: &[Series],
    cfg: &crate::AppConfig,
    header: &str,
    y_bounds: (f64, f64),
    background: Color,
    style: VisualStyle,
) -> io::Result<()> {
    terminal.draw(|frame| {
        if cfg.show_ui {
            let available_height = frame.area().height;
            let header_height = if available_height == 0 {
                0
            } else if available_height == 1 {
                1
            } else {
                wrapped_line_count(header, frame.area().width)
                    .min(usize::from(available_height.saturating_sub(1)))
                    .max(1) as u16
            };
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(header_height), Constraint::Min(0)])
                .split(frame.area());
            let header = Paragraph::new(header)
                .style(Style::default().fg(Color::White).bg(background))
                .wrap(Wrap { trim: true });

            frame.render_widget(header, layout[0]);
            render_chart(frame, layout[1], series, cfg, y_bounds, background, style);
        } else {
            render_chart(
                frame,
                frame.area(),
                series,
                cfg,
                y_bounds,
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
    let area = centered_rect(frame.area(), 54, 24);
    let text = if area.width < 54 || area.height < 24 {
        vec![
            Line::from("q/Ctrl-C/Ctrl-Q quit | Space pause"),
            Line::from("h header | r reset | p preset"),
            Line::from("w waves 3/2/1 | v views"),
            Line::from("↑/↓ gain | ←/→ zoom"),
            Line::from("t original/rgb/mono | RGB hue: 45s"),
            Line::from("b terminal/black/midnight"),
            Line::from("{/} persistence frames -/+ (1..120)"),
            Line::from("Wave, Ocean Mist, Spectrum,"),
            Line::from("Spectrogram, Lissajous, Persistence"),
            Line::from("Esc/? close"),
        ]
    } else {
        vec![
            Line::from("keyboard controls"),
            Line::from(""),
            Line::from("q / Ctrl-C / Ctrl-Q  quit"),
            Line::from("Space              pause / resume"),
            Line::from("h                  hide / show header"),
            Line::from("r                  reset view"),
            Line::from("p                  cycle view preset"),
            Line::from("w                  cycle waves: 3 / 2 / 1"),
            Line::from("v                  cycle audio views"),
            Line::from("↑ / ↓              gain up / down"),
            Line::from("← / →              zoom out / in"),
            Line::from("t                  original / rgb / mono"),
            Line::from("b                  terminal / black / midnight"),
            Line::from("RGB                one hue / 45 seconds"),
            Line::from("ocean-mist          dotted wave layers"),
            Line::from("lissajous          stereo L/R plot, 60 s display rotation"),
            Line::from("persistence        fading time-domain history"),
            Line::from("{ / }              history frames - / + (1..120)"),
            Line::from("spectrum           fractional bars + peak hold"),
            Line::from("spectrogram        old ↑ / now ↓; log Hz axis"),
            Line::from("heat brightness    -65..0 dBFS contrast"),
            Line::from("Esc / ?            close this window"),
        ]
    };

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
    y_bounds: (f64, f64),
    background: Color,
    style: VisualStyle,
) {
    match style {
        VisualStyle::Line => {
            render_line_chart(frame, area, series, cfg, y_bounds.0, y_bounds.1, background)
        }
        VisualStyle::OceanMist => render_ocean_mist(frame, area, series, cfg, background),
        VisualStyle::Bars => render_bars(frame, area, series, cfg, background),
        VisualStyle::Spectrogram => render_spectrogram(frame, area, series, cfg, background),
        VisualStyle::Lissajous => render_lissajous(frame, area, series, cfg, background),
        VisualStyle::Persistence => {
            render_persistence(frame, area, series, cfg, y_bounds.0, y_bounds.1, background)
        }
    }
}

fn trace_color(series: &Series, brightness: f64, ghost: bool) -> Color {
    match (series.palette, ghost) {
        (Palette::Rgb, _) => shade_color(series.color, brightness),
        (Palette::Original, true) | (Palette::Mono, true) => Color::DarkGray,
        (Palette::Original, false) | (Palette::Mono, false) => series.color,
    }
}

fn rotate_lissajous_point(left: f64, right: f64, sin: f64, cos: f64) -> (f64, f64) {
    (left * cos - right * sin, left * sin + right * cos)
}

fn render_lissajous(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    cfg: &crate::AppConfig,
    background: Color,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(background)),
        area,
    );
    if area.is_empty() {
        return;
    }
    let Some(trace) = series.first() else {
        return;
    };
    if trace.xy_points.is_empty() {
        frame.render_widget(
            Paragraph::new(trace.name.as_str())
                .style(Style::default().fg(trace.color).bg(background))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    // Use calibrated text-cell pixels where available, otherwise width:height
    // defaults to 1:2. Braille's 2×4 dots have x/y pitch ratio 2*cell_aspect.
    // Fit ±scale on the shorter physical span and expand only the other axis.
    // Use dot intervals (not cell counts) to match Canvas's endpoint mapping.
    let dot_aspect = cfg.cell_aspect * 2.0;
    let x_span = (f64::from(area.width) * 2.0 - 1.0) * dot_aspect;
    let y_span = f64::from(area.height) * 4.0 - 1.0;
    let common_span = x_span.min(y_span);
    let x_limit = cfg.scale * x_span / common_span;
    let y_limit = cfg.scale * y_span / common_span;
    let canvas = Canvas::default()
        .background_color(background)
        .x_bounds([-x_limit, x_limit])
        .y_bounds([-y_limit, y_limit])
        .marker(Marker::Braille)
        .paint(|ctx| {
            let angle = if cfg.lissajous_rotation.is_finite() {
                cfg.lissajous_rotation
            } else {
                0.0
            };
            let (sin, cos) = angle.sin_cos();
            let mut painter = Painter::from(&mut *ctx);
            // Rotate only the display coordinates; each plotted pair still comes
            // from the measured left/right samples, with no channel synthesis.
            for &(left, right) in &trace.xy_points {
                if left.is_finite() && right.is_finite() {
                    let (x, y) = rotate_lissajous_point(left, right, sin, cos);
                    if let Some((x, y)) = painter.get_point(x, y) {
                        painter.paint(x, y, trace.color);
                    }
                }
            }
            // Direct Painter writes do not mark Context dirty; publish this layer.
            ctx.layer();
        });
    frame.render_widget(canvas, area);
}

fn draw_ocean_guides(frame: &mut Frame, area: Rect, background: Color, lanes: usize) {
    if area.width < 4 || area.height < 4 || lanes == 0 {
        return;
    }
    let style = Style::default()
        .fg(Color::DarkGray)
        .bg(background)
        .add_modifier(ratatui::style::Modifier::DIM);
    for lane in 0..lanes {
        let center = (lane as f64 + 0.5) / lanes as f64;
        let row = ((1.0 - center) * f64::from(area.height - 1)).round() as u16;
        for col in (0..area.width).step_by(4) {
            if let Some(cell) = frame.buffer_mut().cell_mut((area.x + col, area.y + row)) {
                if cell.symbol() == " " {
                    cell.set_symbol("·").set_style(style);
                }
            }
        }
    }
}

fn render_ocean_mist(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    cfg: &crate::AppConfig,
    background: Color,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(background)),
        area,
    );
    let lanes = series.len().min(3);
    if lanes == 0 || area.width == 0 || area.height == 0 {
        return;
    }
    let target_cols = usize::from(area.width.max(2));
    let visible_len = ((target_cols as f64) * cfg.zoom)
        .round()
        .clamp(2.0, cfg.width.max(2) as f64) as usize;
    let lanes_data: Vec<_> = series
        .iter()
        .take(lanes)
        .map(|trace| {
            (
                trace,
                resample_samples(&trace.samples, visible_len, target_cols),
                resample_samples(&trace.peak_levels, visible_len, target_cols),
            )
        })
        .collect();
    let x_bounds = [0.0, target_cols.saturating_sub(1) as f64];
    let dot_stride = (target_cols / 48).max(2);
    let canvas = Canvas::default()
        .background_color(background)
        .x_bounds(x_bounds)
        .y_bounds([0.0, 1.0])
        .marker(Marker::Dot)
        .paint(|ctx| {
            for (lane, (trace, current, previous)) in lanes_data.iter().enumerate() {
                let center = (lane as f64 + 0.5) / lanes as f64;
                let amplitude = 0.16 / cfg.scale.max(0.05);
                for (index, (x, sample)) in previous.iter().enumerate() {
                    if index % dot_stride == 0 || index + 1 == previous.len() {
                        let y = (center + sample * amplitude).clamp(0.0, 1.0);
                        ctx.print(
                            *x,
                            y,
                            Line::from(Span::styled(
                                "·",
                                Style::default().fg(trace_color(trace, 0.18, true)),
                            )),
                        );
                    }
                }
                for (index, (x, sample)) in current.iter().enumerate() {
                    if index % dot_stride == 0 || index + 1 == current.len() {
                        let y = (center + sample * amplitude).clamp(0.0, 1.0);
                        ctx.print(
                            *x,
                            y,
                            Line::from(Span::styled(
                                "•",
                                Style::default().fg(trace_color(trace, 0.72, false)),
                            )),
                        );
                    }
                }
            }
        });
    frame.render_widget(canvas, area);
    draw_ocean_guides(frame, area, background, lanes);
}

/// Leave two bottom rows for frequency ticks and labels when they fit.
fn spectral_plot_area(area: Rect, time_gutter: bool) -> Rect {
    let gutter = if time_gutter && area.width >= 16 {
        4
    } else {
        0
    };
    Rect::new(
        area.x + gutter,
        area.y,
        area.width - gutter,
        area.height
            .saturating_sub(if area.height >= 4 { 2 } else { 0 }),
    )
}

fn render_frequency_axis(
    frame: &mut Frame,
    area: Rect,
    plot: Rect,
    max_frequency: f64,
    background: Color,
) {
    if area.height < 4 || plot.width == 0 || !max_frequency.is_finite() || max_frequency <= 20.0 {
        return;
    }
    let style = Style::default().fg(Color::DarkGray).bg(background);
    let mut label_end = plot.x;
    let mut has_label = false;
    for (frequency, label) in [
        (20.0_f64, "20"),
        (100.0, "100"),
        (1_000.0, "1k"),
        (10_000.0, "10k"),
    ] {
        if frequency > max_frequency || label.len() > usize::from(plot.width) {
            continue;
        }
        let fraction = (frequency / 20.0).ln() / (max_frequency / 20.0).ln();
        let col = ((fraction * f64::from(plot.width)).floor() as u16).min(plot.width - 1);
        let label_width = label.len() as u16;
        let start = (plot.x + col)
            .saturating_sub(label_width / 2)
            .clamp(plot.x, plot.right() - label_width);
        if has_label && start <= label_end {
            continue;
        }
        if let Some(cell) = frame.buffer_mut().cell_mut((plot.x + col, plot.bottom())) {
            cell.set_symbol("┬").set_style(style);
        }
        frame
            .buffer_mut()
            .set_string(start, plot.bottom() + 1, label, style);
        label_end = start + label_width;
        has_label = true;
    }
    if plot.width >= 2 && (!has_label || plot.right() - 2 > label_end) {
        frame
            .buffer_mut()
            .set_string(plot.right() - 2, plot.bottom() + 1, "Hz", style);
    }
}

fn render_bars(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    cfg: &crate::AppConfig,
    background: Color,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(background)),
        area,
    );
    let Some(spectrum) = series.first() else {
        return;
    };
    let plot = spectral_plot_area(area, false);
    // Reduce the old aggressive per-frame lift; retain the live-only reference.
    let reference = spectrum
        .samples
        .iter()
        .copied()
        .fold(SPECTRUM_REFERENCE_FLOOR, f64::max);
    let display_multiplier = SPECTRUM_PEAK_TARGET / reference;
    let spaced_bins = !spectrum.samples.is_empty()
        && usize::from(plot.width) >= spectrum.samples.len().saturating_mul(2);
    if plot.height > 0 {
        let guide_style = Style::default()
            .fg(Color::DarkGray)
            .bg(background)
            .add_modifier(ratatui::style::Modifier::DIM);
        for col in 0..plot.width {
            let bin_start = spaced_bins
                && (usize::from(col) * spectrum.samples.len()) % usize::from(plot.width)
                    < spectrum.samples.len();
            if let Some(cell) = frame
                .buffer_mut()
                .cell_mut((plot.x + col, plot.bottom() - 1))
            {
                cell.set_symbol(if bin_start { "┴" } else { "─" })
                    .set_style(guide_style);
            }
        }
    }
    const EIGHTHS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    for col in 0..plot.width {
        // Keep a one-cell gap only when there is room for at least two cells per bar.
        if spaced_bins
            && col > 0
            && (usize::from(col) * spectrum.samples.len()) % usize::from(plot.width)
                < spectrum.samples.len()
        {
            continue;
        }
        let intensity = (column_level(&spectrum.samples, col, plot.width) * display_multiplier
            / cfg.scale)
            .clamp(0.0, 1.0);
        let height = intensity * f64::from(plot.height);
        let eighths = if height > 0.0 {
            (height * 8.0).round().max(1.0) as u32
        } else {
            0
        };
        let filled_rows = eighths.div_ceil(8) as u16;
        let color = match spectrum.palette {
            Palette::Rgb => shade_color(spectrum.color, intensity),
            Palette::Original => spectrum.color,
            Palette::Mono => {
                let shade = (255.0 * intensity).round() as u8;
                Color::Rgb(shade, shade, shade)
            }
        };
        for row in 0..filled_rows {
            let remaining = eighths.saturating_sub(u32::from(row) * 8).min(8);
            if let Some(cell) = frame
                .buffer_mut()
                .cell_mut((plot.x + col, plot.bottom() - 1 - row))
            {
                cell.set_symbol(EIGHTHS[remaining as usize - 1]).set_style(
                    Style::default()
                        .fg(color)
                        .bg(background)
                        .remove_modifier(ratatui::style::Modifier::DIM),
                );
            }
        }
        let peak = (column_level(&spectrum.peak_levels, col, plot.width) * display_multiplier
            / cfg.scale)
            .clamp(0.0, 1.0);
        let peak_rows = (peak * f64::from(plot.height)).ceil() as u16;
        if peak > intensity && peak_rows > filled_rows && peak_rows <= plot.height {
            let marker_color = match spectrum.palette {
                Palette::Rgb => shade_color(spectrum.color, 0.45),
                Palette::Original | Palette::Mono => Color::DarkGray,
            };
            if let Some(cell) = frame
                .buffer_mut()
                .cell_mut((plot.x + col, plot.bottom() - peak_rows))
            {
                cell.set_symbol("─").set_style(
                    Style::default()
                        .fg(marker_color)
                        .bg(background)
                        .add_modifier(ratatui::style::Modifier::DIM),
                );
            }
        }
    }
    render_frequency_axis(frame, area, plot, spectrum.max_frequency, background);
}

fn render_spectrogram(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    cfg: &crate::AppConfig,
    background: Color,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(background)),
        area,
    );
    let plot = spectral_plot_area(area, true);
    let rows = series.len().min(usize::from(plot.height));
    let first = series.len().saturating_sub(rows);
    let top = plot.bottom().saturating_sub(rows as u16);
    for (row, spectrum) in series[first..].iter().enumerate() {
        for col in 0..plot.width {
            let level =
                spectrogram_intensity(column_level(&spectrum.samples, col, plot.width), cfg.scale);
            let color = if level <= 0.0 {
                background
            } else {
                match spectrum.palette {
                    Palette::Original => heat_color(level, background),
                    Palette::Rgb => shade_color(spectrum.color, level),
                    Palette::Mono => {
                        let shade = (255.0 * level).round() as u8;
                        Color::Rgb(shade, shade, shade)
                    }
                }
            };
            if let Some(cell) = frame
                .buffer_mut()
                .cell_mut((plot.x + col, top + row as u16))
            {
                cell.set_symbol(" ").set_bg(color);
            }
        }
    }
    if plot.x > area.x && rows > 0 {
        let label_style = Style::default().fg(Color::DarkGray).bg(background);
        if rows > 1 {
            frame.render_widget(
                Paragraph::new("old").style(label_style),
                Rect::new(area.x, top, 3, 1),
            );
        }
        frame.render_widget(
            Paragraph::new("now").style(label_style),
            Rect::new(area.x, plot.bottom() - 1, 3, 1),
        );
    }
    let max_frequency = series.last().map_or(0.0, |spectrum| spectrum.max_frequency);
    render_frequency_axis(frame, area, plot, max_frequency, background);
}

fn column_level(samples: &[f64], col: u16, width: u16) -> f64 {
    if samples.is_empty() || width == 0 {
        return 0.0;
    }
    let start = usize::from(col) * samples.len() / usize::from(width);
    let end = ((usize::from(col) + 1) * samples.len() / usize::from(width))
        .max(start + 1)
        .min(samples.len());
    samples[start..end].iter().copied().fold(0.0, f64::max)
}

/// DSP levels encode -80..0 dBFS; discard below -65 dBFS before lifting detail.
fn spectrogram_intensity(sample: f64, scale: f64) -> f64 {
    let scaled = (sample / scale).clamp(0.0, 1.0);
    ((scaled - 0.1875) / 0.8125).clamp(0.0, 1.0).powf(0.7)
}

fn heat_color(level: f64, background: Color) -> Color {
    if level <= 0.0 {
        return background;
    }
    Color::Rgb(
        (255.0 * level.powi(3)).round() as u8,
        (255.0 * level.powf(1.5)).round() as u8,
        (255.0 * (0.12 + 0.88 * level)).round() as u8,
    )
}

/// Stream raw historical frames onto one grid, oldest first. Only the canvas
/// owns a raster buffer; resampling retains at most the previous point.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PersistenceTraceEffect {
    time_zoom: f64,
    amplitude_zoom: f64,
    visible: bool,
}

fn persistence_trace_effect(
    trace_index: usize,
    trace_count: usize,
    band_count: usize,
) -> PersistenceTraceEffect {
    let band_count = band_count.max(1);
    // Anchor complete frames at the newest named bands. This keeps the live
    // waveform untouched even if a caller supplies a partial oldest frame.
    let traces_after = trace_count.saturating_sub(trace_index + 1);
    let age = traces_after / band_count;
    let frame_count = trace_count.div_ceil(band_count);
    let trail = if frame_count > 1 {
        age as f64 / (frame_count - 1) as f64
    } else {
        0.0
    };
    PersistenceTraceEffect {
        // Older traces expand just enough to imply depth without overpowering
        // the current waveform.
        time_zoom: 1.0 + 0.10 * trail,
        amplitude_zoom: 1.0 + 0.05 * trail,
        // Retained high-frequency traces alias into a dense Braille band. Keep
        // only the live treble trace; bass and mid still retain full history.
        visible: !(band_count == 3 && traces_after.is_multiple_of(band_count) && age > 0),
    }
}

fn persistence_sample_position(
    sample_count: usize,
    column: usize,
    column_count: usize,
    time_zoom: f64,
) -> f64 {
    let last_sample = sample_count.saturating_sub(1) as f64;
    let span = last_sample / time_zoom.max(1.0);
    let start = (last_sample - span) / 2.0;
    start + column as f64 * span / column_count.saturating_sub(1).max(1) as f64
}

fn render_persistence(
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
    let y_max = if y_max > y_min { y_max } else { y_min + 1.0 };
    let y_bounds = [y_min * cfg.scale, y_max * cfg.scale];
    let band_count = series
        .iter()
        .rev()
        .take_while(|trace| !trace.name.is_empty())
        .count()
        .max(1);
    let canvas = Canvas::default()
        .background_color(background)
        .x_bounds([0.0, target_cols.saturating_sub(1) as f64])
        .y_bounds(y_bounds)
        .marker(Marker::Braille)
        .paint(|ctx| {
            for (trace_index, trace) in series.iter().enumerate() {
                let effect = persistence_trace_effect(trace_index, series.len(), band_count);
                if !effect.visible {
                    continue;
                }
                let start = trace.samples.len().saturating_sub(visible_len);
                let window = &trace.samples[start..];
                if window.is_empty() {
                    continue;
                }
                let sample_at = |column| {
                    let pos = persistence_sample_position(
                        window.len(),
                        column,
                        target_cols,
                        effect.time_zoom,
                    );
                    let lo = pos.floor() as usize;
                    let hi = pos.ceil() as usize;
                    let fraction = pos - lo as f64;
                    (window[lo] * (1.0 - fraction) + window[hi] * fraction) * effect.amplitude_zoom
                };
                let first = sample_at(0);
                ctx.draw(&Points {
                    coords: &[(0.0, first)],
                    color: trace.color,
                });
                let mut previous = first;
                for col in 1..target_cols {
                    let current = sample_at(col);
                    // CanvasLine rejects either clipped endpoint. Keep an in-range
                    // point when its preceding neighbor lies outside the viewport.
                    if (y_bounds[0]..=y_bounds[1]).contains(&previous) {
                        ctx.draw(&CanvasLine {
                            x1: (col - 1) as f64,
                            y1: previous,
                            x2: col as f64,
                            y2: current,
                            color: trace.color,
                        });
                    } else {
                        ctx.draw(&Points {
                            coords: &[(col as f64, current)],
                            color: trace.color,
                        });
                    }
                    previous = current;
                }
            }
        });
    frame.render_widget(canvas, area);
    render_trace_legend(frame, area, series, background, true);
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

    let wave_start = wave_activity_window_start(series, visible_len);
    let point_sets: Vec<Vec<(f64, f64)>> = series
        .iter()
        .map(|trace| resample_samples_at(&trace.samples, visible_len, target_cols, wave_start))
        .collect();

    // Keep the requested bounds exact: one-sided padding moves the midpoint.
    let y_max = if y_max > y_min { y_max } else { y_min + 1.0 };
    let x_bounds = [0.0, target_cols.saturating_sub(1) as f64];
    let y_bounds = [y_min * cfg.scale, y_max * cfg.scale];
    // Every palette uses one full-area Braille grid. An unlabeled Chart has the
    // same area, but a separate Canvas per dataset; no layout or rasterizer split
    // should move the visual center when the palette changes.
    let canvas = Canvas::default()
        .background_color(background)
        .x_bounds(x_bounds)
        .y_bounds(y_bounds)
        .marker(Marker::Braille)
        .paint(|ctx| {
            for (s, points) in series.iter().zip(&point_sets) {
                ctx.draw(&Points {
                    coords: points,
                    color: s.color,
                });
                for edge in points.windows(2) {
                    ctx.draw(&CanvasLine {
                        x1: edge[0].0,
                        y1: edge[0].1,
                        x2: edge[1].0,
                        y2: edge[1].1,
                        color: s.color,
                    });
                }
            }
        });
    frame.render_widget(canvas, area);
    render_trace_legend(frame, area, series, background, false);
}

fn wave_activity_window_start(series: &[Series], visible_len: usize) -> usize {
    let sample_count = series
        .iter()
        .map(|trace| trace.samples.len())
        .max()
        .unwrap_or(0);
    let visible_len = visible_len.max(2);
    let latest_start = sample_count.saturating_sub(visible_len);

    let peak = series
        .iter()
        .flat_map(|trace| trace.samples.iter().take(sample_count))
        .copied()
        .filter(|sample| sample.is_finite())
        .map(f64::abs)
        .fold(0.0, f64::max);
    if peak == 0.0 {
        return latest_start;
    }

    // Weight all bands together so they keep the same time alignment. Squared
    // relative amplitude centers the visible window on the strongest activity,
    // while rejecting very low-level tails/noise.
    let threshold = 0.05;
    let mut weighted_index = 0.0;
    let mut total_weight = 0.0;
    for trace in series {
        for (index, sample) in trace.samples.iter().take(sample_count).enumerate() {
            if sample.is_finite() {
                let relative = sample.abs() / peak;
                if relative >= threshold {
                    let weight = relative * relative;
                    weighted_index += index as f64 * weight;
                    total_weight += weight;
                }
            }
        }
    }
    if total_weight <= 0.0 {
        return latest_start;
    }

    let activity_center = weighted_index / total_weight;
    let viewport_center = (visible_len - 1) as f64 / 2.0;
    let requested_start = (activity_center - viewport_center).round();
    // Never synthesize a silent tail at either edge. Clamp the shared viewport
    // to available history; edge activity may be less centered, but the rendered
    // endpoints always come from captured samples.
    requested_start.clamp(0.0, latest_start as f64) as usize
}

fn render_trace_legend(
    frame: &mut Frame,
    area: Rect,
    series: &[Series],
    background: Color,
    hide_empty: bool,
) {
    // Keep the legend as an overlay, never a gutter that shifts/scales the plot.
    // Avoid empty Chart datasets: each would allocate another full Braille grid.
    let name_width = series
        .iter()
        .filter(|trace| !hide_empty || !trace.name.is_empty())
        .map(|trace| Span::raw(trace.name.as_str()).width())
        .max()
        .unwrap_or(0);
    let legend_width = name_width + 2;
    let legend_height = series
        .iter()
        .filter(|trace| !hide_empty || !trace.name.is_empty())
        .count()
        + 2;
    // Like Chart, hide oversized legends; still allow a single three-row label
    // in compact plots where the old quarter-height rule could hide its name.
    if name_width == 0
        || legend_width > usize::from(area.width / 4)
        || legend_height > usize::from((area.height / 4).max(3).min(area.height))
    {
        return;
    }
    let legend = Rect::new(
        area.right() - legend_width as u16,
        area.y,
        legend_width as u16,
        legend_height as u16,
    );
    frame.render_widget(Clear, legend);
    frame.render_widget(
        Block::bordered().style(Style::default().fg(Color::DarkGray).bg(background)),
        legend,
    );
    for (row, trace) in series
        .iter()
        .filter(|trace| !hide_empty || !trace.name.is_empty())
        .enumerate()
    {
        frame.buffer_mut().set_string(
            legend.x + 1,
            legend.y + 1 + row as u16,
            trace.name.as_str(),
            Style::default().fg(trace.color).bg(background),
        );
    }
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
            let pos = col as f64 * last_src / denom;
            let lo = pos.floor() as usize;
            let hi = pos.ceil() as usize;
            let fraction = pos - lo as f64;
            let value = window[lo] * (1.0 - fraction) + window[hi] * fraction;
            (col as f64, value)
        })
        .collect()
}

fn resample_samples_at(
    samples: &[f64],
    visible_len: usize,
    target_cols: usize,
    start: usize,
) -> Vec<(f64, f64)> {
    let target_cols = target_cols.max(2);
    let visible_len = visible_len.max(2);
    let start = start.min(samples.len().saturating_sub(visible_len));
    let end = start.saturating_add(visible_len).min(samples.len());
    let window = &samples[start..end];
    let last_offset = window.len().saturating_sub(1) as f64;
    let denom = (target_cols - 1) as f64;
    let sample_at = |offset: usize| {
        window
            .get(offset)
            .copied()
            .filter(|sample| sample.is_finite())
            .unwrap_or(0.0)
    };

    (0..target_cols)
        .map(|col| {
            let position = col as f64 * last_offset / denom;
            let low = position.floor() as usize;
            let high = position.ceil() as usize;
            let fraction = position - low as f64;
            let value = sample_at(low) * (1.0 - fraction) + sample_at(high) * fraction;
            (col as f64, value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    /// Decode actual Braille bits into physical dot coordinates (origin top-left).
    fn braille_pixels(buffer: &ratatui::buffer::Buffer) -> Vec<(i32, i32)> {
        let mut pixels = Vec::new();
        let offsets = [
            (0, 0),
            (0, 1),
            (0, 2),
            (1, 0),
            (1, 1),
            (1, 2),
            (0, 3),
            (1, 3),
        ];
        for y in buffer.area.top()..buffer.area.bottom() {
            for x in buffer.area.left()..buffer.area.right() {
                let Some(symbol) = buffer[(x, y)].symbol().chars().next() else {
                    continue;
                };
                if !('\u{2800}'..='\u{28ff}').contains(&symbol) {
                    continue;
                }
                let bits = symbol as u32 - 0x2800;
                for (bit, (dx, dy)) in offsets.iter().enumerate() {
                    if bits & (1 << bit) != 0 {
                        pixels.push((i32::from(x) * 2 + dx, i32::from(y) * 4 + dy));
                    }
                }
            }
        }
        pixels.sort_unstable();
        pixels
    }

    fn pixel_bounds(pixels: &[(i32, i32)]) -> (i32, i32, i32, i32) {
        (
            pixels.iter().map(|p| p.0).min().expect("visible x"),
            pixels.iter().map(|p| p.0).max().expect("visible x"),
            pixels.iter().map(|p| p.1).min().expect("visible y"),
            pixels.iter().map(|p| p.1).max().expect("visible y"),
        )
    }

    const GEOMETRY_HEADER: &str = "Media: playing\nview | gain | zoom";

    fn plot_geometry(width: u16, height: u16, show_ui: bool) -> (f64, f64, f64) {
        // Both explicit header lines fit even the compact fixture.
        let top = if show_ui { 2.0 } else { 0.0 };
        let x_span = f64::from(width) * 2.0 - 1.0;
        let y_span = (f64::from(height) - top) * 4.0 - 1.0;
        (x_span / 2.0, top * 4.0 + y_span / 2.0, x_span.min(y_span))
    }

    fn xy_series(points: Vec<(f64, f64)>, palette: Palette) -> Series {
        Series {
            name: "XY unavailable: mono input".to_string(),
            color: Color::Rgb(255, 85, 0),
            palette,
            // Deliberately unrelated scalar data must never become an XY trace.
            samples: vec![1.0, -1.0, 1.0, -1.0],
            peak_levels: vec![1.0; 4],
            max_frequency: 0.0,
            xy_points: points,
        }
    }

    fn xy_buffer(
        width: u16,
        height: u16,
        points: Vec<(f64, f64)>,
        palette: Palette,
        scale: f64,
        show_ui: bool,
    ) -> io::Result<ratatui::buffer::Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        let cfg = crate::AppConfig {
            show_ui,
            scale,
            // XY must not use the scalar time-window/resampling settings.
            width: 2,
            zoom: 32.0,
            ..Default::default()
        };
        draw_frame(
            &mut terminal,
            &[xy_series(points, palette)],
            &cfg,
            GEOMETRY_HEADER,
            (-17.0, 83.0),
            Color::Rgb(0, 8, 24),
            VisualStyle::Lissajous,
        )?;
        Ok(terminal.backend().buffer().clone())
    }

    fn persistence_trace(samples: Vec<f64>, color: Color, palette: Palette) -> Series {
        let mut trace = xy_series(Vec::new(), palette);
        trace.name.clear();
        trace.samples = samples;
        trace.color = color;
        trace
    }

    fn time_domain_buffer(
        width: u16,
        height: u16,
        history: &[Series],
        cfg: &crate::AppConfig,
        style: VisualStyle,
    ) -> io::Result<ratatui::buffer::Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        draw_frame(
            &mut terminal,
            history,
            cfg,
            GEOMETRY_HEADER,
            (-1.0, 1.0),
            Color::Rgb(0, 8, 24),
            style,
        )?;
        Ok(terminal.backend().buffer().clone())
    }

    #[test]
    fn persistence_trails_zoom_subtly_and_only_live_treble_is_visible() -> io::Result<()> {
        let oldest_bass = persistence_trace_effect(0, 12, 3);
        let oldest_treble = persistence_trace_effect(2, 12, 3);
        let newest_treble = persistence_trace_effect(11, 12, 3);
        assert_eq!(oldest_bass.time_zoom, 1.10);
        assert_eq!(oldest_bass.amplitude_zoom, 1.05);
        assert!(oldest_bass.visible);
        assert!(!oldest_treble.visible);
        assert_eq!(
            newest_treble,
            PersistenceTraceEffect {
                time_zoom: 1.0,
                amplitude_zoom: 1.0,
                visible: true,
            }
        );

        // Age is measured from the newest frame, so even an incomplete oldest
        // frame cannot accidentally zoom or hide current bands.
        assert!(!persistence_trace_effect(0, 4, 3).visible);
        for trace_index in 1..4 {
            assert_eq!(
                persistence_trace_effect(trace_index, 4, 3),
                PersistenceTraceEffect {
                    time_zoom: 1.0,
                    amplitude_zoom: 1.0,
                    visible: true,
                }
            );
        }

        let normal_start = persistence_sample_position(40, 0, 48, 1.0);
        let normal_end = persistence_sample_position(40, 47, 48, 1.0);
        let zoomed_start = persistence_sample_position(40, 0, 48, oldest_bass.time_zoom);
        let zoomed_end = persistence_sample_position(40, 47, 48, oldest_bass.time_zoom);
        assert_eq!((normal_start, normal_end), (0.0, 39.0));
        assert!(zoomed_start > 1.0);
        assert!(zoomed_end < 38.0);
        assert!((zoomed_start + zoomed_end - 39.0).abs() < f64::EPSILON);

        let cfg = crate::AppConfig {
            show_ui: false,
            zoom: 1.0,
            width: 48,
            ..Default::default()
        };
        let samples: Vec<_> = (0..48)
            .map(|column| if column % 2 == 0 { -0.7 } else { 0.7 })
            .collect();
        let mut current = vec![
            persistence_trace(Vec::new(), Color::Cyan, Palette::Original),
            persistence_trace(Vec::new(), Color::Green, Palette::Original),
            persistence_trace(samples.clone(), Color::Blue, Palette::Original),
        ];
        for (trace, name) in current.iter_mut().zip(["bass", "mid", "treble"]) {
            trace.name = name.to_string();
        }
        let live = time_domain_buffer(48, 12, &current, &cfg, VisualStyle::Persistence)?;

        let mut with_empty_history = vec![
            persistence_trace(Vec::new(), Color::Cyan, Palette::Original),
            persistence_trace(Vec::new(), Color::Green, Palette::Original),
            persistence_trace(Vec::new(), Color::Blue, Palette::Original),
        ];
        with_empty_history.extend(current.clone());
        assert_eq!(
            live,
            time_domain_buffer(48, 12, &with_empty_history, &cfg, VisualStyle::Persistence,)?,
            "history effects changed newest waveform geometry"
        );

        let mut history = vec![
            persistence_trace(Vec::new(), Color::Cyan, Palette::Original),
            persistence_trace(Vec::new(), Color::Green, Palette::Original),
            persistence_trace(samples, Color::Blue, Palette::Original),
        ];
        history.extend(current);
        let hidden_history = time_domain_buffer(48, 12, &history, &cfg, VisualStyle::Persistence)?;
        assert_eq!(
            hidden_history, live,
            "historical treble should not add any persistence pixels"
        );
        Ok(())
    }

    #[test]
    fn persistence_retains_distinct_traces_and_newest_color_wins_crossings() -> io::Result<()> {
        let cfg = crate::AppConfig {
            show_ui: false,
            zoom: 1.0,
            ..Default::default()
        };
        let old_color = Color::Rgb(51, 0, 0);
        let new_color = Color::Rgb(0, 255, 0);
        for palette in [Palette::Original, Palette::Rgb, Palette::Mono] {
            let old = persistence_trace(vec![0.0; 48], old_color, palette);
            let new = persistence_trace(
                (0..48).map(|col| -0.75 + 1.5 * col as f64 / 47.0).collect(),
                new_color,
                palette,
            );
            let old_only = time_domain_buffer(
                48,
                12,
                std::slice::from_ref(&old),
                &cfg,
                VisualStyle::Persistence,
            )?;
            let new_only = time_domain_buffer(
                48,
                12,
                std::slice::from_ref(&new),
                &cfg,
                VisualStyle::Persistence,
            )?;
            let history = [old, new];
            let combined = time_domain_buffer(48, 12, &history, &cfg, VisualStyle::Persistence)?;
            let mut crossing_cells = 0;
            let mut retained_cells = 0;
            for ((old, new), combined) in old_only
                .content
                .iter()
                .zip(&new_only.content)
                .zip(&combined.content)
            {
                if new.fg == new_color {
                    assert_eq!(combined.fg, new_color, "newest trace owns its cells");
                    if old.fg == old_color {
                        crossing_cells += 1;
                    }
                } else if old.fg == old_color {
                    assert_eq!(combined.fg, old_color, "old trace remains outside overlap");
                    retained_cells += 1;
                }
            }
            assert!(crossing_cells > 0);
            assert!(retained_cells > 0);
            let reversed = [history[1].clone(), history[0].clone()];
            let reverse = time_domain_buffer(48, 12, &reversed, &cfg, VisualStyle::Persistence)?;
            for (old, reverse) in old_only.content.iter().zip(&reverse.content) {
                if old.fg == old_color {
                    assert_eq!(reverse.fg, old_color, "slice order controls overwrite");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn persistence_is_resize_safe_without_mutating_history() -> io::Result<()> {
        let history = [
            persistence_trace(vec![-0.5; 512], Color::Rgb(51, 0, 0), Palette::Original),
            persistence_trace(
                (0..512).map(|i| (i as f64 * 0.04).sin() * 0.5).collect(),
                Color::Rgb(0, 255, 0),
                Palette::Rgb,
            ),
        ];
        let raw_samples: Vec<_> = history.iter().map(|trace| trace.samples.clone()).collect();
        for (width, height) in [(48, 12), (84, 26), (187, 52)] {
            for show_ui in [false, true] {
                for (zoom, sample_width, scale) in [(0.5, 512, 1.0), (4.0, 100, 2.0)] {
                    let cfg = crate::AppConfig {
                        show_ui,
                        zoom,
                        width: sample_width,
                        scale,
                        ..Default::default()
                    };
                    let persistence = time_domain_buffer(
                        width,
                        height,
                        &history,
                        &cfg,
                        VisualStyle::Persistence,
                    )?;
                    let wave =
                        time_domain_buffer(width, height, &history, &cfg, VisualStyle::Line)?;
                    assert_eq!(persistence.area, wave.area);
                    assert_eq!(persistence.area, Rect::new(0, 0, width, height));
                    assert!(!braille_pixels(&persistence).is_empty());
                    assert!(!braille_pixels(&wave).is_empty());
                    for (trace, original) in history.iter().zip(&raw_samples) {
                        assert_eq!(&trace.samples, original);
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn persistence_empty_and_singleton_frames_have_no_phantom_baseline() -> io::Result<()> {
        for (width, height) in [(0, 0), (1, 1), (2, 2), (48, 12), (187, 52)] {
            for palette in [Palette::Original, Palette::Rgb, Palette::Mono] {
                let cfg = crate::AppConfig {
                    show_ui: false,
                    scale: 2.0,
                    ..Default::default()
                };
                let empty = persistence_trace(Vec::new(), Color::Red, palette);
                let empty_buffer = time_domain_buffer(
                    width,
                    height,
                    std::slice::from_ref(&empty),
                    &cfg,
                    VisualStyle::Persistence,
                )?;
                assert!(braille_pixels(&empty_buffer).is_empty());
                let no_history =
                    time_domain_buffer(width, height, &[], &cfg, VisualStyle::Persistence)?;
                assert_eq!(empty_buffer, no_history);
                let singleton = persistence_trace(vec![1.0], Color::Rgb(9, 81, 27), palette);
                let buffer = time_domain_buffer(
                    width,
                    height,
                    &[empty, singleton],
                    &cfg,
                    VisualStyle::Persistence,
                )?;
                if width > 0 && height > 0 {
                    let pixels = braille_pixels(&buffer);
                    let (left, right, top, bottom) = pixel_bounds(&pixels);
                    assert_eq!((left, right), (0, i32::from(width) * 2 - 1));
                    assert_eq!(top, bottom);
                    // +1 in scaled bounds [-2, 2] lies exactly one quarter down.
                    assert_eq!(top, ((f64::from(height) * 4.0 - 1.0) * 0.25).floor() as i32);
                    assert!(buffer
                        .content
                        .iter()
                        .any(|cell| cell.fg == Color::Rgb(9, 81, 27)));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn persistence_legend_uses_only_named_current_bands() -> io::Result<()> {
        let cfg = crate::AppConfig {
            show_ui: false,
            ..Default::default()
        };
        let mut history: Vec<_> = (0..24)
            .map(|_| persistence_trace(vec![-0.5; 48], Color::Rgb(51, 0, 0), Palette::Original))
            .collect();
        let mut newest = persistence_trace(vec![0.0; 48], Color::Green, Palette::Original);
        newest.name = "Wave".to_string();
        history.push(newest);
        let buffer = time_domain_buffer(84, 26, &history, &cfg, VisualStyle::Persistence)?;
        let legend: String = (79..83).map(|x| buffer[(x, 1)].symbol()).collect();
        assert_eq!(legend, "Wave");
        assert_eq!(buffer[(79, 1)].fg, Color::Green);
        Ok(())
    }

    #[test]
    fn persistence_preserves_in_bounds_points_between_clipped_neighbors() -> io::Result<()> {
        for (width, height) in [(48, 12), (187, 52)] {
            let cfg = crate::AppConfig {
                show_ui: false,
                zoom: 1.0,
                width: usize::from(width),
                scale: 0.25,
                ..Default::default()
            };
            let trace = persistence_trace(
                (0..width)
                    .map(|col| if col % 2 == 0 { 0.8 } else { 0.0 })
                    .collect(),
                Color::Rgb(0, 255, 0),
                Palette::Rgb,
            );
            let wave = time_domain_buffer(
                width,
                height,
                std::slice::from_ref(&trace),
                &cfg,
                VisualStyle::Line,
            )?;
            assert_eq!(braille_pixels(&wave).len(), usize::from(width / 2));
            let persistence =
                time_domain_buffer(width, height, &[trace], &cfg, VisualStyle::Persistence)?;
            let persistence_pixels = braille_pixels(&persistence);
            assert!(
                persistence_pixels.len() >= usize::from(width / 2),
                "persistence must retain each in-range sample between clipped neighbors"
            );
        }
        Ok(())
    }

    #[test]
    fn wave_activity_window_centers_energy_and_keeps_all_bands_time_aligned() {
        let mut bass = vec![0.0; 128];
        let mut mid = vec![0.0; 128];
        for sample in &mut bass[20..40] {
            *sample = 1.0;
        }
        for sample in &mut mid[50..60] {
            *sample = 0.25;
        }
        let history = [
            persistence_trace(bass, Color::Green, Palette::Original),
            persistence_trace(mid, Color::Cyan, Palette::Original),
        ];
        let start = wave_activity_window_start(&history, 32);
        let total_weight = 20.0 + 10.0 * 0.25_f64.powi(2);
        let expected_center = (20..40).map(|i| i as f64).sum::<f64>() / 20.0 * 20.0
            + (50..60).map(|i| i as f64).sum::<f64>() / 10.0 * 10.0 * 0.25_f64.powi(2);
        let expected_center = expected_center / total_weight;
        assert!(((start as f64 + 15.5) - expected_center).abs() <= 1.0);

        let selected = resample_samples_at(&history[0].samples, 32, 32, start);
        let first = start as usize;
        assert_eq!(selected.first(), Some(&(0.0, history[0].samples[first])));
        assert_eq!(
            selected.last(),
            Some(&(31.0, history[0].samples[first + 31]))
        );
    }

    #[test]
    fn wave_activity_window_clamps_edge_bursts_without_zero_padding() {
        for (range, expected_start, expected_center) in [(0..8, 0, 3.5), (120..128, 96, 27.5)] {
            let mut samples = vec![0.0; 128];
            for sample in &mut samples[range] {
                *sample = 1.0;
            }
            let history = [persistence_trace(samples, Color::Green, Palette::Original)];
            let start = wave_activity_window_start(&history, 32);
            assert_eq!(start, expected_start);

            let visible = resample_samples_at(&history[0].samples, 32, 32, start);
            let active_columns: Vec<_> = visible
                .iter()
                .filter(|(_, sample)| *sample > 0.5)
                .map(|(column, _)| *column)
                .collect();
            let center = active_columns.iter().sum::<f64>() / active_columns.len() as f64;
            assert!((center - expected_center).abs() < 0.01);
            assert_eq!(
                visible.first().map(|(_, sample)| *sample),
                Some(history[0].samples[start])
            );
            assert_eq!(
                visible.last().map(|(_, sample)| *sample),
                Some(history[0].samples[start + 31])
            );
        }
    }

    #[test]
    fn wave_activity_window_handles_a_longer_aligned_band_without_overrunning_shorter_data() {
        let short = persistence_trace(vec![0.0; 64], Color::Green, Palette::Original);
        let mut long_samples = vec![0.0; 256];
        for sample in &mut long_samples[170..190] {
            *sample = 1.0;
        }
        let long = persistence_trace(long_samples, Color::Cyan, Palette::Original);
        let history = [short, long];
        let start = wave_activity_window_start(&history, 32);
        assert_eq!(start, 164);

        let short_window = resample_samples_at(&history[0].samples, 32, 32, start);
        assert!(short_window.iter().all(|(_, sample)| *sample == 0.0));
        let long_window = resample_samples_at(&history[1].samples, 32, 32, start);
        assert!(long_window.iter().any(|(_, sample)| *sample > 0.5));
    }

    #[test]
    fn wave_renderer_centers_a_localized_audio_burst_horizontally() -> io::Result<()> {
        let mut samples = vec![0.0; 512];
        for (offset, sample) in samples[250..270].iter_mut().enumerate() {
            *sample = 0.8 * (offset as f64 * 0.5).sin();
        }
        let trace = persistence_trace(samples, Color::Green, Palette::Original);
        let cfg = crate::AppConfig {
            show_ui: false,
            ..Default::default()
        };
        let buffer = time_domain_buffer(80, 16, &[trace], &cfg, VisualStyle::Line)?;
        let (_, center_y, _) = plot_geometry(80, 16, false);
        let off_baseline: Vec<_> = braille_pixels(&buffer)
            .into_iter()
            .filter(|(_, y)| (f64::from(*y) - center_y).abs() > 2.0)
            .collect();
        assert!(!off_baseline.is_empty());
        let energy_center = off_baseline.iter().map(|(x, _)| f64::from(*x)).sum::<f64>()
            / off_baseline.len() as f64;
        assert!(
            (energy_center - 79.5).abs() <= 5.0,
            "center={energy_center}"
        );
        Ok(())
    }

    #[test]
    fn wave_activity_window_centers_continuous_signal_and_keeps_silence_at_latest_edge() {
        let continuous = [persistence_trace(
            vec![0.5; 128],
            Color::Green,
            Palette::Original,
        )];
        assert_eq!(wave_activity_window_start(&continuous, 32), 48);

        let silence = [persistence_trace(
            vec![0.0; 128],
            Color::Green,
            Palette::Original,
        )];
        assert_eq!(wave_activity_window_start(&silence, 32), 96);

        let mut tiny = vec![0.0; 128];
        tiny[40..48].fill(f64::MIN_POSITIVE);
        let tiny_signal = [persistence_trace(tiny, Color::Green, Palette::Original)];
        assert_eq!(wave_activity_window_start(&tiny_signal, 32), 28);
    }

    #[test]
    fn wave_zero_and_symmetric_levels_share_full_plot_geometry_for_every_palette() -> io::Result<()>
    {
        for (width, height) in [(48, 12), (84, 26), (187, 52)] {
            for show_ui in [false, true] {
                for palette in [Palette::Original, Palette::Rgb, Palette::Mono] {
                    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
                    let cfg = crate::AppConfig {
                        show_ui,
                        ..Default::default()
                    };
                    let (_, center_y, _) = plot_geometry(width, height, show_ui);
                    let mut levels = Vec::new();
                    for level in [-0.5, 0.0, 0.5] {
                        let mut trace = xy_series(Vec::new(), palette);
                        trace.name = "Wave".to_string();
                        trace.samples = vec![level; 512];
                        draw_frame(
                            &mut terminal,
                            &[trace],
                            &cfg,
                            GEOMETRY_HEADER,
                            (-1.0, 1.0),
                            Color::Rgb(0, 8, 24),
                            VisualStyle::Line,
                        )?;
                        let buffer = terminal.backend().buffer();
                        let pixels = braille_pixels(buffer);
                        let (left, right, top, bottom) = pixel_bounds(&pixels);
                        assert_eq!(left, 0);
                        assert!(right >= i32::from(width), "trace must span the plot");
                        assert_eq!(top, bottom, "constant Wave must stay horizontal");
                        // The unchanged top-right legend may mask an upper trace
                        // on compact plots, but never moves its rasterized y.
                        if level == 0.0 {
                            assert_eq!(right, i32::from(width) * 2 - 1);
                            assert_eq!(pixels.len(), usize::from(width) * 2);
                            assert_eq!(f64::from(top), center_y.floor());
                        }
                        levels.push(top);
                        assert!(buffer
                            .content
                            .iter()
                            .all(|cell| cell.bg == Color::Rgb(0, 8, 24)));
                    }
                    assert!(
                        (f64::from(levels[0] + levels[2]) - center_y * 2.0).abs() <= 1.0,
                        "symmetric input must straddle the remaining plot center"
                    );
                    let mut trace = xy_series(Vec::new(), palette);
                    trace.name = "Wave".to_string();
                    trace.samples = (0..width)
                        .map(|i| {
                            let phase = std::f64::consts::TAU * f64::from(i) / f64::from(width - 1);
                            0.6 * phase.sin()
                        })
                        .collect();
                    let cycle_cfg = crate::AppConfig {
                        zoom: 1.0,
                        ..cfg.clone()
                    };
                    draw_frame(
                        &mut terminal,
                        &[trace],
                        &cycle_cfg,
                        GEOMETRY_HEADER,
                        (-1.0, 1.0),
                        Color::Rgb(0, 8, 24),
                        VisualStyle::Line,
                    )?;
                    let pixels = braille_pixels(terminal.backend().buffer());
                    let (left, right, top, bottom) = pixel_bounds(&pixels);
                    assert_eq!((left, right), (0, i32::from(width) * 2 - 1));
                    assert!(
                        (f64::from(top + bottom) - center_y * 2.0).abs() <= 1.01,
                        "a complete zero-mean cycle must remain centered after rasterization"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn lissajous_rotation_transforms_stereo_coordinates_without_synthesizing_channels() {
        let (sin, cos) = std::f64::consts::FRAC_PI_2.sin_cos();
        let (x, y) = rotate_lissajous_point(1.0, 0.0, sin, cos);
        assert!(x.abs() < 1e-12);
        assert!((y - 1.0).abs() < 1e-12);
        let (x, y) = rotate_lissajous_point(0.0, -1.0, sin, cos);
        assert!((x - 1.0).abs() < 1e-12);
        assert!(y.abs() < 1e-12);
    }

    #[test]
    fn lissajous_equal_and_opposed_channels_rasterize_as_centered_diagonals() -> io::Result<()> {
        for (width, height) in [(48, 12), (187, 52)] {
            for show_ui in [false, true] {
                for palette in [Palette::Original, Palette::Rgb, Palette::Mono] {
                    for sign in [-1.0, 1.0] {
                        let points = (0..=1024)
                            .map(|i| {
                                let left = -0.75 + 1.5 * f64::from(i) / 1024.0;
                                (left, sign * left)
                            })
                            .collect();
                        let buffer = xy_buffer(width, height, points, palette, 1.0, show_ui)?;
                        let pixels = braille_pixels(&buffer);
                        let (cx, cy, span) = plot_geometry(width, height, show_ui);
                        let (left, right, top, bottom) = pixel_bounds(&pixels);
                        assert!(f64::from(right - left) >= span * 0.75 - 2.0);
                        assert!((right - left - (bottom - top)).abs() <= 1);
                        assert!((f64::from(left + right) / 2.0 - cx).abs() <= 1.0);
                        assert!((f64::from(top + bottom) / 2.0 - cy).abs() <= 1.0);
                        for (x, y) in pixels {
                            let dx = f64::from(x) - cx;
                            let dy = cy - f64::from(y);
                            assert!((dy - sign * dx).abs() <= 1.01);
                            let cell = &buffer[((x / 2) as u16, (y / 4) as u16)];
                            assert_eq!(cell.fg, Color::Rgb(255, 85, 0));
                            assert_eq!(cell.bg, Color::Rgb(0, 8, 24));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn lissajous_quadrature_circle_and_unequal_channel_ellipse_keep_physical_aspect(
    ) -> io::Result<()> {
        for (width, height) in [(48, 12), (187, 52)] {
            for show_ui in [false, true] {
                for right_amplitude in [0.75, 0.35] {
                    let points = (0..4096)
                        .map(|i| {
                            let phase = std::f64::consts::TAU * f64::from(i) / 4096.0;
                            (0.75 * phase.cos(), right_amplitude * phase.sin())
                        })
                        .collect();
                    let buffer = xy_buffer(width, height, points, Palette::Original, 1.0, show_ui)?;
                    let pixels = braille_pixels(&buffer);
                    let (cx, cy, span) = plot_geometry(width, height, show_ui);
                    let rx = span * 0.75 / 2.0;
                    let ry = span * right_amplitude / 2.0;
                    let (left, right, top, bottom) = pixel_bounds(&pixels);
                    assert!((f64::from(right - left) - rx * 2.0).abs() <= 2.0);
                    assert!((f64::from(bottom - top) - ry * 2.0).abs() <= 2.0);
                    assert!((f64::from(left + right) / 2.0 - cx).abs() <= 1.0);
                    assert!((f64::from(top + bottom) / 2.0 - cy).abs() <= 1.0);
                    for (x, y) in pixels {
                        let dx = (f64::from(x) - cx) / rx;
                        let dy = (cy - f64::from(y)) / ry;
                        assert!(
                            (dx * dx + dy * dy - 1.0).abs() <= 2.5 / rx + 2.5 / ry,
                            "actual rasterized dots must remain on the measured ellipse"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn lissajous_calibrated_cell_aspects_preserve_physical_circles_and_center() -> io::Result<()> {
        for cell_aspect in [0.4, 0.5, 0.75, 1.0] {
            let mut terminal = Terminal::new(TestBackend::new(120, 30))?;
            let cfg = crate::AppConfig {
                show_ui: false,
                cell_aspect,
                ..Default::default()
            };
            let points = (0..4096)
                .map(|index| {
                    let phase = std::f64::consts::TAU * f64::from(index) / 4096.0;
                    (0.75 * phase.cos(), 0.75 * phase.sin())
                })
                .collect();
            draw_frame(
                &mut terminal,
                &[xy_series(points, Palette::Original)],
                &cfg,
                "",
                (-1.0, 1.0),
                Color::Black,
                VisualStyle::Lissajous,
            )?;
            let pixels = braille_pixels(terminal.backend().buffer());
            let (left, right, top, bottom) = pixel_bounds(&pixels);
            let dot_aspect = cell_aspect * 2.0;
            let physical_width = f64::from(right - left) * dot_aspect;
            let physical_height = f64::from(bottom - top);
            assert!((physical_width - physical_height).abs() <= 2.0 * (1.0 + dot_aspect));
            assert!((f64::from(left + right) / 2.0 - 119.5).abs() <= 1.0);
            assert!((f64::from(top + bottom) / 2.0 - 59.5).abs() <= 1.0);
        }
        Ok(())
    }

    #[test]
    fn lissajous_two_to_one_channels_draw_two_lobes_and_a_center_crossing() -> io::Result<()> {
        for (width, height) in [(48, 12), (187, 52)] {
            let points = (0..4096)
                .map(|i| {
                    let phase = std::f64::consts::TAU * f64::from(i) / 4096.0;
                    (0.75 * (2.0 * phase).sin(), 0.75 * phase.sin())
                })
                .collect();
            let buffer = xy_buffer(width, height, points, Palette::Rgb, 1.0, false)?;
            let pixels = braille_pixels(&buffer);
            let (cx, cy, span) = plot_geometry(width, height, false);
            let radius = span * 0.75 / 2.0;
            let mut quadrants = [false; 4];
            let mut center = false;
            for (x, y) in pixels {
                let dx = (f64::from(x) - cx) / radius;
                let dy = (cy - f64::from(y)) / radius;
                // x = sin(2t), y = sin(t), tested against rasterized dots.
                assert!((dx * dx - 4.0 * dy * dy * (1.0 - dy * dy)).abs() <= 14.0 / radius);
                if dx.abs() > 0.3 && dy.abs() > 0.3 {
                    quadrants[usize::from(dx > 0.0) + 2 * usize::from(dy > 0.0)] = true;
                }
                if dx.abs() < 2.0 / radius && dy.abs() < 2.0 / radius {
                    center = true;
                }
                if dy.abs() < 0.05 {
                    assert!(
                        dx.abs() < 0.2,
                        "no fake horizontal baseline at the crossing"
                    );
                }
            }
            assert!(quadrants.into_iter().all(|occupied| occupied));
            assert!(center, "figure-eight must cross the stable zero center");
        }
        Ok(())
    }

    #[test]
    fn lissajous_silence_is_one_center_dot_and_common_gain_scales_both_raw_channels(
    ) -> io::Result<()> {
        for (width, height) in [(48, 12), (187, 52)] {
            for show_ui in [false, true] {
                let (cx, cy, span) = plot_geometry(width, height, show_ui);
                for scale in [0.5, 1.0, 2.0] {
                    let buffer = xy_buffer(
                        width,
                        height,
                        vec![(0.0, 0.0); 512],
                        Palette::Mono,
                        scale,
                        show_ui,
                    )?;
                    assert_eq!(
                        braille_pixels(&buffer),
                        vec![(cx.floor() as i32, cy.floor() as i32)],
                        "silence is not a time-domain baseline"
                    );
                    // A DC pair stays off-center; neither channel is demeaned or normalized.
                    let buffer = xy_buffer(
                        width,
                        height,
                        vec![(0.3, -0.2)],
                        Palette::Mono,
                        scale,
                        show_ui,
                    )?;
                    let pixels = braille_pixels(&buffer);
                    assert_eq!(pixels.len(), 1);
                    let expected_x = cx + 0.3 * span / (2.0 * scale);
                    let expected_y = cy + 0.2 * span / (2.0 * scale);
                    assert!((f64::from(pixels[0].0) - expected_x).abs() < 1.01);
                    assert!((f64::from(pixels[0].1) - expected_y).abs() < 1.01);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn lissajous_plots_only_actual_finite_in_bounds_pairs_without_connecting_them() -> io::Result<()>
    {
        for (width, height) in [(48, 12), (187, 52)] {
            let points = vec![
                (-0.5, -0.5),
                (0.5, 0.5),
                (-10_000.0, 0.0),
                (0.0, 10_000.0),
                (f64::NAN, 0.0),
                (0.0, f64::INFINITY),
            ];
            let buffer = xy_buffer(width, height, points, Palette::Rgb, 1.0, false)?;
            let pixels = braille_pixels(&buffer);
            assert_eq!(
                pixels.len(),
                2,
                "no interpolation, clipping smear, or fake curve"
            );
            let (cx, cy, _) = plot_geometry(width, height, false);
            assert!(pixels.iter().all(|&(x, y)| {
                (f64::from(x) - cx).abs() > 2.0 && (f64::from(y) - cy).abs() > 2.0
            }));
        }
        Ok(())
    }

    #[test]
    fn lissajous_empty_data_never_falls_back_to_scalar_or_other_series() -> io::Result<()> {
        for (width, height) in [(48, 12), (187, 52)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            let cfg = crate::AppConfig {
                show_ui: false,
                ..Default::default()
            };
            let series = [
                xy_series(Vec::new(), Palette::Original),
                xy_series(vec![(-1.0, 1.0), (1.0, -1.0)], Palette::Original),
            ];
            draw_frame(
                &mut terminal,
                &series,
                &cfg,
                "",
                (-1.0, 1.0),
                Color::Black,
                VisualStyle::Lissajous,
            )?;
            let buffer = terminal.backend().buffer();
            assert!(braille_pixels(buffer).is_empty());
            draw_frame(
                &mut terminal,
                &[],
                &cfg,
                "",
                (-1.0, 1.0),
                Color::Black,
                VisualStyle::Lissajous,
            )?;
            assert!(terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|cell| { cell.symbol() == " " && cell.bg == Color::Black }));
        }
        Ok(())
    }

    #[test]
    fn lissajous_tiny_and_empty_surfaces_keep_actual_point_semantics() -> io::Result<()> {
        for (width, height) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
            let buffer = xy_buffer(
                width,
                height,
                vec![(0.0, 0.0)],
                Palette::Original,
                1.0,
                false,
            )?;
            assert_eq!(braille_pixels(&buffer).len(), 1);
        }
        let mut terminal = Terminal::new(TestBackend::new(1, 1))?;
        terminal.draw(|frame| {
            render_lissajous(
                frame,
                Rect::new(0, 0, 0, 0),
                &[xy_series(vec![(0.0, 0.0)], Palette::Original)],
                &crate::AppConfig::default(),
                Color::Black,
            );
        })?;
        assert!(braille_pixels(terminal.backend().buffer()).is_empty());
        Ok(())
    }

    #[test]
    fn ocean_mist_renders_distinct_rgb_wave_lanes_across_its_viewport() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(96, 24))?;
        let cfg = crate::AppConfig {
            show_ui: false,
            width: 128,
            ..Default::default()
        };
        let hue = Color::Rgb(255, 128, 0);
        let brightness = [1.0, 0.75, 0.5];
        let series: Vec<_> = brightness
            .into_iter()
            .zip([0.55, -0.45, 0.35])
            .map(|(level, sample)| Series {
                name: String::new(),
                color: shade_color(hue, level),
                palette: Palette::Rgb,
                samples: vec![sample; 128],
                peak_levels: vec![-sample; 128],
                max_frequency: 0.0,
                xy_points: Vec::new(),
            })
            .collect();
        draw_frame(
            &mut terminal,
            &series,
            &cfg,
            "",
            (0.0, 1.0),
            Color::Black,
            VisualStyle::OceanMist,
        )?;
        let buffer = terminal.backend().buffer();
        let trace_colors: std::collections::BTreeSet<_> = buffer
            .content
            .iter()
            .filter(|cell| cell.symbol() == "•")
            .filter_map(|cell| match cell.fg {
                Color::Rgb(r, g, b) if r != 0 || g != 0 || b != 0 => Some((r, g, b)),
                _ => None,
            })
            .collect();
        assert!(
            trace_colors.len() >= 3,
            "all three hue-brightness lanes should appear"
        );
        assert!(buffer
            .content
            .iter()
            .any(|cell| cell.symbol() == "·" && cell.fg == Color::DarkGray));
        let has_spacing = (0..24).any(|row| {
            let dots: Vec<_> = (0..96)
                .filter(|&col| buffer[(col, row)].symbol() == "•")
                .collect();
            dots.windows(2).any(|pair| pair[1] > pair[0] + 1)
        });
        assert!(has_spacing, "wave samples should render as separated dots");
        for col in [0, 95] {
            assert!(
                (0..24).any(|row| buffer[(col, row)].symbol() == "•"),
                "ocean dot missing at viewport edge {col}"
            );
        }
        Ok(())
    }

    #[test]
    fn ocean_mist_handles_empty_and_tiny_surfaces() -> io::Result<()> {
        let cfg = crate::AppConfig::default();
        for (width, height) in [(1, 1), (2, 2), (8, 3), (24, 8)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| {
                let area = frame.area();
                super::render_chart(
                    frame,
                    area,
                    &[],
                    &cfg,
                    (-1.0, 1.0),
                    Color::Black,
                    VisualStyle::OceanMist,
                );
            })?;
            assert!(terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|cell| cell.bg == Color::Black));
        }
        Ok(())
    }

    #[test]
    fn ocean_mist_resizes_with_small_and_wide_terminals() -> io::Result<()> {
        let cfg = crate::AppConfig {
            width: 512,
            ..Default::default()
        };
        let series = [Series {
            name: String::new(),
            color: Color::Cyan,
            palette: Palette::Original,
            samples: (0..512)
                .map(|index| (index as f64 * 0.031).sin() * 0.7)
                .collect(),
            peak_levels: vec![0.0; 512],
            max_frequency: 0.0,
            xy_points: Vec::new(),
        }];
        let header = "Media: playing\ntheme rgb | bg terminal | view ocean-mist | layers 3 | bass 0.12 mid 0.34 treble 0.56 | peak 0.72 rms 0.18 clip 0%";
        for (width, height) in [(48, 12), (84, 26), (187, 52)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            draw_frame(
                &mut terminal,
                &series,
                &cfg,
                header,
                (0.0, 1.0),
                Color::Black,
                VisualStyle::OceanMist,
            )?;
            let buffer = terminal.backend().buffer();
            let dots: Vec<_> = buffer
                .content
                .iter()
                .enumerate()
                .filter(|(_, cell)| cell.symbol() == "•")
                .map(|(index, _)| index)
                .collect();
            assert!(!dots.is_empty(), "Ocean Mist vanished at {width}×{height}");
            assert!(
                dots.iter()
                    .all(|&index| index < usize::from(width) * usize::from(height)),
                "dot escaped terminal bounds at {width}×{height}"
            );
        }
        Ok(())
    }

    #[test]
    fn spectrum_keeps_narrow_frequency_peaks_visible_in_small_terminals() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(2, 3))?;
        let cfg = crate::AppConfig {
            show_ui: false,
            scale: 1.0,
            ..Default::default()
        };
        let series = vec![Series {
            name: "spectrum".to_string(),
            color: Color::Cyan,
            palette: Palette::Original,
            samples: vec![0.0, 1.0, 0.0, 0.5],
            peak_levels: vec![0.0, 1.0, 0.0, 0.5],
            max_frequency: 24_000.0,
            xy_points: Vec::new(),
        }];
        draw_frame(
            &mut terminal,
            &series,
            &cfg,
            "",
            (0.0, 1.0),
            Color::Black,
            VisualStyle::Bars,
        )?;
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), " ");
        assert_eq!(buffer[(0, 1)].symbol(), "█");
        assert_eq!(buffer[(0, 2)].symbol(), "█");
        assert_eq!(buffer[(1, 0)].symbol(), " ");
        assert_eq!(buffer[(1, 1)].symbol(), " ");
        assert_eq!(buffer[(1, 2)].symbol(), "█");
        Ok(())
    }

    fn spectrogram_row(samples: Vec<f64>, palette: Palette, color: Color) -> Series {
        Series {
            name: String::new(),
            color,
            palette,
            samples,
            peak_levels: Vec::new(),
            max_frequency: 24_000.0,
            xy_points: Vec::new(),
        }
    }

    #[test]
    fn spectrogram_newest_advances_upward_and_crops_oldest_rows() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(16, 5))?;
        let cfg = crate::AppConfig::default();
        let background = Color::Rgb(0, 8, 24);
        let mut rows: Vec<_> = [0.25, 0.5, 0.75]
            .into_iter()
            .map(|level| spectrogram_row(vec![level; 12], Palette::Original, Color::Cyan))
            .collect();
        terminal.draw(|frame| {
            let area = frame.area();
            render_spectrogram(frame, area, &rows, &cfg, background);
        })?;
        let former_newest = terminal.backend().buffer()[(4, 2)].bg;
        assert_eq!(
            former_newest,
            heat_color(spectrogram_intensity(0.75, 1.0), background)
        );
        rows.push(spectrogram_row(
            vec![1.0; 12],
            Palette::Original,
            Color::Cyan,
        ));
        terminal.draw(|frame| {
            let area = frame.area();
            render_spectrogram(frame, area, &rows, &cfg, background);
        })?;
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(4, 1)].bg, former_newest);
        assert_eq!(
            buffer[(4, 0)].bg,
            heat_color(spectrogram_intensity(0.5, 1.0), background)
        );
        assert_eq!(buffer[(4, 2)].bg, Color::Rgb(255, 255, 255));
        let old: String = (0..3).map(|x| buffer[(x, 0)].symbol()).collect();
        let now: String = (0..3).map(|x| buffer[(x, 2)].symbol()).collect();
        assert_eq!(old, "old");
        assert_eq!(now, "now");
        // Evicting a retained row outside the visible crop cannot change the plot.
        let before = buffer.clone();
        rows.remove(0);
        terminal.draw(|frame| {
            let area = frame.area();
            render_spectrogram(frame, area, &rows, &cfg, background);
        })?;
        assert_eq!(terminal.backend().buffer(), &before);
        Ok(())
    }

    #[test]
    fn tiny_surfaces_keep_media_header_without_chart_underflow() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(8, 1))?;
        draw_frame(
            &mut terminal,
            &[],
            &crate::AppConfig::default(),
            "Brave\nPlaying",
            (0.0, 1.0),
            Color::Black,
            VisualStyle::Bars,
        )?;
        let buffer = terminal.backend().buffer();
        let text: String = (0..5).map(|x| buffer[(x, 0)].symbol()).collect();
        assert_eq!(text, "Brave");
        Ok(())
    }

    #[test]
    fn rgb_time_mapping_is_saturated_slow_and_wraps_after_forty_five_seconds() {
        use std::time::Duration;
        for (millis, color) in [
            (0, Color::Rgb(255, 0, 0)),
            (7500, Color::Rgb(255, 255, 0)),
            (15000, Color::Rgb(0, 255, 0)),
            (22500, Color::Rgb(0, 255, 255)),
            (30000, Color::Rgb(0, 0, 255)),
            (37500, Color::Rgb(255, 0, 255)),
        ] {
            assert_eq!(rgb_at_time(Duration::from_millis(millis)), color);
            assert_eq!(rgb_at_time(Duration::from_millis(millis + 45000)), color);
        }
        assert_eq!(rgb_at_time(Duration::from_secs(1)), Color::Rgb(255, 34, 0));
        assert_eq!(
            rgb_at_time(Duration::from_millis(100)),
            Color::Rgb(255, 3, 0)
        );
        assert_eq!(
            shade_color(Color::Rgb(255, 128, 0), 0.5),
            Color::Rgb(128, 64, 0)
        );
        assert_eq!(
            shade_color(Color::Rgb(255, 128, 0), -1.0),
            Color::Rgb(0, 0, 0)
        );
        assert_eq!(
            shade_color(Color::Rgb(255, 128, 0), 2.0),
            Color::Rgb(255, 128, 0)
        );
    }

    #[test]
    fn rgb_waveforms_use_uniform_supplied_hue_and_trace_brightness() -> io::Result<()> {
        use std::collections::BTreeMap;

        let cfg = crate::AppConfig {
            show_ui: false,
            width: 128,
            ..Default::default()
        };
        let hue = rgb_at_time(std::time::Duration::from_secs(5));
        let colors = [1.0, 0.7, 0.4].map(|brightness| shade_color(hue, brightness));
        for count in 1..=3 {
            let mut terminal = Terminal::new(TestBackend::new(96, 24))?;
            let series: Vec<_> = [0.65, 0.0, -0.65][..count]
                .iter()
                .enumerate()
                .map(|(index, &level)| Series {
                    name: String::new(),
                    color: colors[index],
                    palette: Palette::Rgb,
                    samples: vec![level; 128],
                    peak_levels: Vec::new(),
                    max_frequency: 0.0,
                    xy_points: Vec::new(),
                })
                .collect();
            draw_frame(
                &mut terminal,
                &series,
                &cfg,
                "",
                (-1.0, 1.0),
                Color::Black,
                VisualStyle::Line,
            )?;
            let buffer = terminal.backend().buffer();
            let mut rows = BTreeMap::<_, Vec<_>>::new();
            for y in 0..24 {
                for x in 0..96 {
                    let cell = &buffer[(x, y)];
                    if cell
                        .symbol()
                        .chars()
                        .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
                    {
                        rows.entry(y).or_default().push((x, cell.fg));
                    }
                }
            }
            assert_eq!(rows.len(), count);
            for (index, row) in rows.values().enumerate() {
                assert!(
                    row.len() >= 94,
                    "trace must stay connected across its width"
                );
                assert!(row.iter().all(|(_, color)| *color == colors[index]));
                for adjacent in row.windows(2) {
                    assert_eq!(
                        adjacent[1].0,
                        adjacent[0].0 + 1,
                        "gap at a segment boundary"
                    );
                }
            }
        }
        Ok(())
    }

    // Measure rendered bar pixels, not the normalization implementation.
    fn bar_eighths(buffer: &ratatui::buffer::Buffer, col: u16, height: u16) -> u16 {
        (0..height)
            .map(|row| match buffer[(col, row)].symbol() {
                "▁" => 1,
                "▂" => 2,
                "▃" => 3,
                "▄" => 4,
                "▅" => 5,
                "▆" => 6,
                "▇" => 7,
                "█" => 8,
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn spectrum_spreads_music_across_small_and_wide_plots_without_clipping_neighbors(
    ) -> io::Result<()> {
        let cfg = crate::AppConfig::default();
        let background = Color::Rgb(0, 8, 24);
        let hue = Color::Rgb(255, 85, 0);
        for (width, height, columns, expected_eighths) in [
            (12, 8, [1, 6, 10], [21, 25, 31]),
            (192, 30, [25, 97, 169], [97, 116, 146]),
        ] {
            for palette in [Palette::Original, Palette::Rgb, Palette::Mono] {
                let mut terminal = Terminal::new(TestBackend::new(width, height))?;
                let mut samples = vec![0.0; 64];
                samples[8] = 0.5;
                samples[32] = 0.6;
                samples[56] = 0.75; // -20 dBFS in the DSP's -80..0 encoding.
                let series = [Series {
                    name: String::new(),
                    color: hue,
                    palette,
                    samples,
                    peak_levels: Vec::new(),
                    max_frequency: 24_000.0,
                    xy_points: Vec::new(),
                }];
                terminal.draw(|frame| {
                    let area = frame.area();
                    render_bars(frame, area, &series, &cfg, background);
                })?;
                let buffer = terminal.backend().buffer();
                let plot_height = height - 2;
                for col in [0, width - 1] {
                    let guide = &buffer[(col, plot_height - 1)];
                    assert!(matches!(guide.symbol(), "─" | "┴"));
                    assert_eq!(guide.fg, Color::DarkGray);
                    assert_eq!(guide.bg, background);
                    assert!(guide.modifier.contains(ratatui::style::Modifier::DIM));
                }
                for (index, col) in columns.into_iter().enumerate() {
                    assert_eq!(
                        bar_eighths(buffer, col, plot_height),
                        expected_eighths[index]
                    );
                    let base = &buffer[(col, plot_height - 1)];
                    assert_eq!(base.bg, background);
                    assert!(!base.modifier.contains(ratatui::style::Modifier::DIM));
                    let intensity = [13.0 / 30.0, 0.52, 0.65][index];
                    assert_eq!(
                        base.fg,
                        match palette {
                            Palette::Original => hue,
                            Palette::Rgb => shade_color(hue, intensity),
                            Palette::Mono => {
                                let shade = (255.0 * intensity).round() as u8;
                                Color::Rgb(shade, shade, shade)
                            }
                        }
                    );
                }
                // Fractional pixels retain headroom even when the short plot reaches its top row.
                assert!(columns.iter().all(|col| buffer[(*col, 0)].symbol() != "█"));
                if width == 192 {
                    assert_eq!(buffer[(24, plot_height - 1)].symbol(), "┴");
                    assert_eq!(buffer[(24, plot_height - 1)].fg, Color::DarkGray);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn silent_spectrum_keeps_only_dim_full_width_baseline_and_sparse_bin_notches() -> io::Result<()>
    {
        let cfg = crate::AppConfig::default();
        let background = Color::Rgb(0, 8, 24);
        for (width, height) in [(12, 8), (192, 30)] {
            for palette in [Palette::Original, Palette::Rgb, Palette::Mono] {
                let mut terminal = Terminal::new(TestBackend::new(width, height))?;
                let series = [Series {
                    name: String::new(),
                    color: Color::Rgb(255, 85, 0),
                    palette,
                    samples: vec![0.0; 64],
                    peak_levels: vec![0.0; 64],
                    max_frequency: 24_000.0,
                    xy_points: Vec::new(),
                }];
                terminal.draw(|frame| {
                    let area = frame.area();
                    render_bars(frame, area, &series, &cfg, background);
                })?;
                let buffer = terminal.backend().buffer();
                let plot_height = height - 2;
                let mut notches = 0;
                for col in 0..width {
                    assert_eq!(bar_eighths(buffer, col, plot_height), 0);
                    assert!((0..plot_height - 1).all(|row| buffer[(col, row)].symbol() == " "));
                    let guide = &buffer[(col, plot_height - 1)];
                    assert!(matches!(guide.symbol(), "─" | "┴"));
                    assert_eq!(guide.fg, Color::DarkGray);
                    assert_eq!(guide.bg, background);
                    assert!(guide.modifier.contains(ratatui::style::Modifier::DIM));
                    if guide.symbol() == "┴" {
                        notches += 1;
                    }
                }
                assert_eq!(notches, if width == 192 { 64 } else { 0 });
            }
        }
        Ok(())
    }

    #[test]
    fn spectrum_first_and_last_frequency_bins_reach_terminal_edges() -> io::Result<()> {
        for (width, height) in [(12, 8), (192, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            let mut samples = vec![0.0; 64];
            samples[0] = 0.5;
            samples[63] = 0.75;
            let series = [Series {
                name: String::new(),
                color: Color::Cyan,
                palette: Palette::Original,
                samples,
                peak_levels: Vec::new(),
                max_frequency: 24_000.0,
                xy_points: Vec::new(),
            }];
            terminal.draw(|frame| {
                let area = frame.area();
                render_bars(
                    frame,
                    area,
                    &series,
                    &crate::AppConfig::default(),
                    Color::Black,
                );
            })?;
            let buffer = terminal.backend().buffer();
            let plot_height = height - 2;
            let left = bar_eighths(buffer, 0, plot_height);
            let right = bar_eighths(buffer, width - 1, plot_height);
            assert!(
                left > 0 && left < right,
                "both edge bins must occupy their actual columns"
            );
            let full_height = plot_height * 8;
            assert!(right >= full_height * 64 / 100 && right <= full_height * 66 / 100 + 1);
            assert_eq!(buffer[(0, plot_height - 1)].fg, Color::Cyan);
            assert_eq!(buffer[(width - 1, plot_height - 1)].fg, Color::Cyan);
        }
        Ok(())
    }

    #[test]
    fn spectrum_gain_remains_effective_and_weak_signals_are_not_normalized_to_full_height(
    ) -> io::Result<()> {
        let background = Color::Black;
        for palette in [Palette::Original, Palette::Rgb, Palette::Mono] {
            let mut terminal = Terminal::new(TestBackend::new(12, 30))?;
            for (level, scale, expected_eighths) in [
                (0.75, 0.5, 224),
                (0.75, 1.0, 146),
                (0.75, 2.0, 73),
                (0.75, 4.0, 36),
                (0.5, 1.0, 146),
                (0.25, 1.0, 73),
                (0.1, 1.0, 29),
                (0.01, 1.0, 3),
                (0.0, 1.0, 0),
            ] {
                let cfg = crate::AppConfig {
                    scale,
                    ..Default::default()
                };
                let series = [Series {
                    name: String::new(),
                    color: Color::Cyan,
                    palette,
                    samples: vec![level; 64],
                    peak_levels: Vec::new(),
                    max_frequency: 24_000.0,
                    xy_points: Vec::new(),
                }];
                terminal.draw(|frame| {
                    let area = frame.area();
                    render_bars(frame, area, &series, &cfg, background);
                })?;
                let buffer = terminal.backend().buffer();
                for col in [0, 6, 11] {
                    assert_eq!(bar_eighths(buffer, col, 28), expected_eighths);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn spectrum_uses_all_fractional_glyphs_without_painting_the_background() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(8, 3))?;
        let cfg = crate::AppConfig {
            show_ui: false,
            ..Default::default()
        };
        let background = Color::Rgb(0, 8, 24);
        let hue = Color::Rgb(255, 85, 0);
        let series = [Series {
            name: String::new(),
            color: hue,
            palette: Palette::Rgb,
            samples: (1..=8)
                .map(|eighth| f64::from(eighth) / (24.0 * 1.3))
                .collect(),
            peak_levels: Vec::new(),
            max_frequency: 24_000.0,
            xy_points: Vec::new(),
        }];
        draw_frame(
            &mut terminal,
            &series,
            &cfg,
            "",
            (0.0, 1.0),
            background,
            VisualStyle::Bars,
        )?;
        let buffer = terminal.backend().buffer();
        for (col, glyph) in ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"].iter().enumerate()
        {
            assert_eq!(buffer[(col as u16, 2)].symbol(), *glyph);
            assert_eq!(buffer[(col as u16, 2)].bg, background);
            assert_eq!(
                buffer[(col as u16, 2)].fg,
                shade_color(hue, (col + 1) as f64 / 24.0)
            );
            assert_eq!(buffer[(col as u16, 1)].symbol(), " ");
        }
        Ok(())
    }

    #[test]
    fn spectrum_peaks_are_downsampled_and_never_overwrite_live_bars() -> io::Result<()> {
        let cfg = crate::AppConfig {
            show_ui: false,
            ..Default::default()
        };
        for palette in [Palette::Original, Palette::Rgb, Palette::Mono] {
            let mut terminal = Terminal::new(TestBackend::new(2, 3))?;
            let hue = Color::Rgb(255, 85, 0);
            let series = [Series {
                name: String::new(),
                color: if palette == Palette::Original {
                    Color::Cyan
                } else {
                    hue
                },
                palette,
                samples: vec![0.0, 0.5 / 1.3, 0.0, 0.25 / 1.3],
                peak_levels: vec![0.0, 1.0 / 1.3, 0.0, 0.25 / 1.3],
                max_frequency: 24_000.0,
                xy_points: Vec::new(),
            }];
            draw_frame(
                &mut terminal,
                &series,
                &cfg,
                "",
                (0.0, 1.0),
                Color::Black,
                VisualStyle::Bars,
            )?;
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(0, 0)].symbol(), "─");
            assert!(buffer[(0, 0)]
                .modifier
                .contains(ratatui::style::Modifier::DIM));
            assert_eq!(
                buffer[(0, 0)].fg,
                if palette == Palette::Rgb {
                    shade_color(hue, 0.45)
                } else {
                    Color::DarkGray
                }
            );
            assert_eq!(buffer[(0, 1)].symbol(), "▄");
            assert_eq!(buffer[(0, 2)].symbol(), "█");
            assert_eq!(buffer[(1, 2)].symbol(), "▆");
            assert_eq!(buffer[(1, 1)].symbol(), " ");
            // The normalized half-level fixture can round either side of a half-byte.
            match (palette, buffer[(0, 2)].fg) {
                (Palette::Original, Color::Cyan) => {}
                (Palette::Rgb, Color::Rgb(r, g, 0)) => {
                    assert!(r.abs_diff(128) <= 1);
                    assert!(g.abs_diff(43) <= 1);
                }
                (Palette::Mono, Color::Rgb(r, g, b)) => {
                    assert!(r.abs_diff(128) <= 1);
                    assert_eq!(r, g);
                    assert_eq!(g, b);
                }
                (_, color) => panic!("incorrect normalized bar palette: {color:?}"),
            }
        }
        Ok(())
    }

    #[test]
    fn unchanged_peak_does_not_create_a_marker_at_a_fractional_rounding_boundary() -> io::Result<()>
    {
        let mut terminal = Terminal::new(TestBackend::new(1, 3))?;
        let cfg = crate::AppConfig {
            show_ui: false,
            scale: 1.0,
            ..Default::default()
        };
        let series = [Series {
            name: String::new(),
            color: Color::Cyan,
            palette: Palette::Original,
            samples: vec![1.01 / (3.0 * 1.3)],
            peak_levels: vec![1.01 / (3.0 * 1.3)],
            max_frequency: 24_000.0,
            xy_points: Vec::new(),
        }];
        draw_frame(
            &mut terminal,
            &series,
            &cfg,
            "",
            (0.0, 1.0),
            Color::Black,
            VisualStyle::Bars,
        )?;
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 2)].symbol(), "█");
        assert_eq!(
            buffer[(0, 1)].symbol(),
            " ",
            "steady signal must not create a detached hold marker"
        );
        Ok(())
    }

    #[test]
    fn spectrum_leaves_gaps_and_air_beneath_detached_peaks() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(4, 6))?;
        let cfg = crate::AppConfig {
            show_ui: false,
            ..Default::default()
        };
        let series = [Series {
            name: String::new(),
            color: Color::Cyan,
            palette: Palette::Original,
            samples: vec![0.375 / 1.3, 0.375 / 1.3],
            peak_levels: vec![1.0 / 1.3, 1.0 / 1.3],
            max_frequency: 24_000.0,
            xy_points: Vec::new(),
        }];
        draw_frame(
            &mut terminal,
            &series,
            &cfg,
            "",
            (0.0, 1.0),
            Color::Black,
            VisualStyle::Bars,
        )?;
        let buffer = terminal.backend().buffer();
        for col in [0, 1, 3] {
            assert_eq!(buffer[(col, 0)].symbol(), "─");
            assert_eq!(buffer[(col, 1)].symbol(), " ");
            assert_eq!(buffer[(col, 2)].symbol(), "▄");
            assert_eq!(buffer[(col, 3)].symbol(), "█");
        }
        assert!((0..3).all(|row| buffer[(2, row)].symbol() == " "));
        assert_eq!(buffer[(2, 3)].symbol(), "┴");
        assert_eq!(buffer[(2, 3)].fg, Color::DarkGray);
        assert!(buffer[(2, 3)]
            .modifier
            .contains(ratatui::style::Modifier::DIM));
        Ok(())
    }

    #[test]
    fn frequency_axis_uses_log_positions_and_omits_above_nyquist() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(80, 6))?;
        let area = Rect::new(0, 0, 80, 6);
        let plot = spectral_plot_area(area, false);
        assert_eq!(plot, Rect::new(0, 0, 80, 4));
        terminal.draw(|frame| render_frequency_axis(frame, area, plot, 20_000.0, Color::Black))?;
        let buffer = terminal.backend().buffer();
        for frequency in [20.0_f64, 100.0, 1_000.0, 10_000.0] {
            let col = ((frequency / 20.0).ln() / 1_000.0_f64.ln() * 80.0).floor() as u16;
            assert_eq!(buffer[(col, 4)].symbol(), "┬");
        }
        let labels: String = (0..80).map(|x| buffer[(x, 5)].symbol()).collect();
        assert!(
            labels.contains("20")
                && labels.contains("100")
                && labels.contains("1k")
                && labels.contains("10k")
                && labels.contains("Hz")
        );
        terminal.draw(|frame| render_frequency_axis(frame, area, plot, 800.0, Color::Black))?;
        let labels: String = (0..80)
            .map(|x| terminal.backend().buffer()[(x, 5)].symbol())
            .collect();
        assert!(labels.contains("20") && labels.contains("100"));
        assert!(!labels.contains("1k") && !labels.contains("10k"));
        Ok(())
    }

    #[test]
    fn spectral_layout_and_axes_are_safe_on_tiny_and_empty_surfaces() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(20, 6))?;
        let series = [Series {
            name: String::new(),
            color: Color::Cyan,
            palette: Palette::Original,
            samples: vec![0.5],
            peak_levels: vec![1.0],
            max_frequency: 24_000.0,
            xy_points: Vec::new(),
        }];
        for width in 0..=20 {
            for height in 0..=6 {
                let area = Rect::new(0, 0, width, height);
                let plot = spectral_plot_area(area, true);
                assert_eq!(plot.x, if width >= 16 { 4 } else { 0 });
                assert_eq!(plot.height, if height >= 4 { height - 2 } else { height });
                terminal.draw(|frame| {
                    render_bars(
                        frame,
                        area,
                        &series,
                        &crate::AppConfig::default(),
                        Color::Black,
                    )
                })?;
                if height >= 4 {
                    let labels: String = (0..width)
                        .map(|x| terminal.backend().buffer()[(x, height - 1)].symbol())
                        .collect();
                    assert!(
                        !labels.contains("20100")
                            && !labels.contains("1001k")
                            && !labels.contains("1k10k")
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn spectrogram_rgb_uses_one_frame_hue_for_all_levels_and_history() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(7, 3))?;
        let cfg = crate::AppConfig::default();
        let background = Color::Rgb(0, 8, 24);
        let levels = vec![0.0, 0.1875, 0.25, 0.5, 0.75, 1.0, 0.5];
        let mut rows: Vec<_> = (0..3)
            .map(|_| spectrogram_row(levels.clone(), Palette::Rgb, Color::Rgb(255, 128, 0)))
            .collect();
        for hue in [Color::Rgb(255, 128, 0), Color::Rgb(0, 255, 128)] {
            for row in &mut rows {
                row.color = hue;
            }
            terminal.draw(|frame| {
                let area = frame.area();
                render_spectrogram(frame, area, &rows, &cfg, background);
            })?;
            let buffer = terminal.backend().buffer();
            assert!(buffer
                .content
                .iter()
                .all(|cell| !matches!(cell.fg, Color::Rgb(..))));
            for y in 0..3 {
                for (x, &level) in levels.iter().enumerate() {
                    let visible = spectrogram_intensity(level, cfg.scale);
                    let expected = if visible == 0.0 {
                        background
                    } else {
                        shade_color(hue, visible)
                    };
                    assert_eq!(buffer[(x as u16, y)].bg, expected);
                    assert_eq!(buffer[(x as u16, y)].symbol(), " ");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn spectrogram_contrast_discards_noise_and_lifts_fixed_range() {
        for level in [-1.0, 0.0, 0.1, 0.1875] {
            assert_eq!(spectrogram_intensity(level, 1.0), 0.0);
        }
        assert!(spectrogram_intensity(0.2, 1.0) > 0.0);
        assert_eq!(spectrogram_intensity(1.0, 1.0), 1.0);
        assert_eq!(spectrogram_intensity(2.0, 1.0), 1.0);
        assert_eq!(
            spectrogram_intensity(0.5, 2.0),
            spectrogram_intensity(0.25, 1.0)
        );
        let middle = spectrogram_intensity(0.5, 1.0);
        assert!((middle - ((0.5_f64 - 0.1875) / 0.8125).powf(0.7)).abs() < 1e-12);
        assert!(middle > (0.5 - 0.1875) / 0.8125);
        let background = Color::Rgb(0, 8, 24);
        assert_eq!(heat_color(0.0, background), background);
        assert_eq!(heat_color(1.0, background), Color::Rgb(255, 255, 255));
        let mut previous = (0, 0, 0);
        for level in [0.01, 0.25, 0.5, 0.75, 1.0] {
            let Color::Rgb(r, g, b) = heat_color(level, background) else {
                panic!("visible heat must use RGB");
            };
            assert!(r >= previous.0 && g >= previous.1 && b >= previous.2);
            assert!(b >= g && g >= r);
            previous = (r, g, b);
        }
    }

    #[test]
    fn spectrogram_mono_separates_weak_mid_and_full_levels_above_the_floor() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(5, 1))?;
        let cfg = crate::AppConfig::default();
        let background = Color::Rgb(0, 8, 24);
        let rows = [spectrogram_row(
            vec![0.0, 0.1875, 0.25, 0.5, 1.0],
            Palette::Mono,
            Color::Gray,
        )];
        terminal.draw(|frame| {
            let area = frame.area();
            render_spectrogram(frame, area, &rows, &cfg, background);
        })?;
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].bg, background);
        assert_eq!(buffer[(1, 0)].bg, background);
        let Color::Rgb(weak, wg, wb) = buffer[(2, 0)].bg else {
            panic!("weak signal must be visible");
        };
        let Color::Rgb(middle, mg, mb) = buffer[(3, 0)].bg else {
            panic!("mid-level signal must be visible");
        };
        assert_eq!((weak, weak), (wg, wb));
        assert_eq!((middle, middle), (mg, mb));
        assert!(
            (35..=60).contains(&weak),
            "-60 dBFS must remain visible but dark"
        );
        assert!(
            (120..=145).contains(&middle),
            "-40 dBFS must occupy useful midtones"
        );
        assert!(
            middle > weak * 2,
            "weak and mid-level signals need clear contrast"
        );
        assert_eq!(buffer[(4, 0)].bg, Color::Rgb(255, 255, 255));
        Ok(())
    }

    #[test]
    fn spectrogram_frequency_axis_is_sparse_logarithmic_and_nyquist_limited() -> io::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(80, 6))?;
        let cfg = crate::AppConfig::default();
        let mut rows = [spectrogram_row(
            vec![0.0; 76],
            Palette::Original,
            Color::Cyan,
        )];
        rows[0].max_frequency = 8_000.0;
        terminal.draw(|frame| {
            let area = frame.area();
            render_spectrogram(frame, area, &rows, &cfg, Color::Black);
        })?;
        let buffer = terminal.backend().buffer();
        let labels: String = (4..80).map(|x| buffer[(x, 5)].symbol()).collect();
        assert!(labels.contains("20") && labels.contains("100") && labels.contains("1k"));
        assert!(!labels.contains("10k"));
        for frequency in [20.0_f64, 100.0, 1_000.0] {
            let col = ((frequency / 20.0).ln() / (8_000.0_f64 / 20.0).ln() * 76.0).floor() as u16;
            assert_ne!(buffer[(4 + col.min(75), 4)].symbol(), " ");
        }
        assert!((4..80).filter(|&x| buffer[(x, 4)].symbol() != " ").count() <= 4);
        Ok(())
    }

    #[test]
    fn spectrogram_handles_empty_and_tiny_surfaces() -> io::Result<()> {
        let cfg = crate::AppConfig::default();
        let rows = [spectrogram_row(
            vec![0.0],
            Palette::Rgb,
            Color::Rgb(255, 0, 0),
        )];
        for (width, height) in [(1, 1), (2, 2), (3, 4), (16, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| {
                render_spectrogram(frame, Rect::new(0, 0, 0, 0), &[], &cfg, Color::Black);
                let area = frame.area();
                render_spectrogram(frame, area, &rows, &cfg, Color::Black);
            })?;
            assert!(terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|cell| cell.bg == Color::Black));
        }
        Ok(())
    }
}
