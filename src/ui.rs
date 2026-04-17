use crate::app::{App, fmt_bytes, fmt_params};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Paragraph, Row, Sparkline, Table, TableState,
    },
};

const LOADED_COLOR: Color = Color::Green;
const UNLOADED_COLOR: Color = Color::DarkGray;
const ACTIVE_COLOR: Color = Color::Cyan;
const HEADER_COLOR: Color = Color::Yellow;
const ERROR_COLOR: Color = Color::Red;
const TITLE_COLOR: Color = Color::LightCyan;
const DIM_COLOR: Color = Color::DarkGray;

pub fn draw(f: &mut Frame, app: &App) {
    let size = f.area();

    // Top-level vertical layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Min(10),    // main content
            Constraint::Length(3),  // footer / help
        ])
        .split(size);

    draw_header(f, app, chunks[0]);
    draw_main(f, app, chunks[1]);
    draw_footer(f, chunks[2]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let loaded_count = app.loaded_models.len();
    let total_count = app.all_models.len();

    let dot = if app.refreshing {
        Span::styled(" ● ", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(" ○ ", Style::default().fg(Color::DarkGray))
    };

    let error_span = if let Some(ref e) = app.error {
        Span::styled(format!("  ⚠ {e}"), Style::default().fg(ERROR_COLOR))
    } else {
        Span::raw("")
    };

    let title_line = Line::from(vec![
        Span::styled(
            " ⚡ llama-monitor ",
            Style::default()
                .fg(TITLE_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", app.server_url),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(" {loaded_count}/{total_count} loaded "),
            Style::default().fg(LOADED_COLOR),
        ),
        dot,
        error_span,
    ]);

    let para = Paragraph::new(title_line)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Left);
    f.render_widget(para, area);
}

fn draw_main(f: &mut Frame, app: &App, area: Rect) {
    // Split into left (models list) and right (detail panels)
    let has_loaded = !app.loaded_models.is_empty();

    if has_loaded {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);

        draw_models_list(f, app, cols[0]);
        draw_loaded_detail(f, app, cols[1]);
    } else {
        draw_models_list(f, app, area);
    }
}

fn draw_models_list(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = ["Status", "Model", "Params", "Size"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows: Vec<Row> = app
        .all_models
        .iter()
        .map(|m| {
            let is_loaded = m.is_loaded();
            let status_cell = if is_loaded {
                Cell::from("● loaded").style(Style::default().fg(LOADED_COLOR))
            } else {
                Cell::from("○ unloaded").style(Style::default().fg(UNLOADED_COLOR))
            };

            let short_id = m.id.clone();
            let id_style = if is_loaded {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(UNLOADED_COLOR)
            };

            // Look up metadata from loaded_models
            let (params_str, size_str) = if let Some(lm) = app
                .loaded_models
                .iter()
                .find(|lm| lm.model_id == m.id)
            {
                let p = lm
                    .meta
                    .as_ref()
                    .and_then(|m| m.n_params)
                    .map(fmt_params)
                    .unwrap_or_default();
                let s = lm
                    .meta
                    .as_ref()
                    .and_then(|m| m.size)
                    .map(fmt_bytes)
                    .unwrap_or_default();
                (p, s)
            } else {
                (String::new(), String::new())
            };

            let tps = app.model_tps(&m.id);
            let tps_span = if tps > 0.1 {
                format!("{tps:.1} t/s")
            } else {
                params_str
            };

            Row::new(vec![
                status_cell,
                Cell::from(short_id).style(id_style),
                Cell::from(tps_span).style(Style::default().fg(ACTIVE_COLOR)),
                Cell::from(size_str).style(Style::default().fg(DIM_COLOR)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Models ")
            .title_style(Style::default().fg(TITLE_COLOR).add_modifier(Modifier::BOLD)),
    )
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = TableState::default();
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_loaded_detail(f: &mut Frame, app: &App, area: Rect) {
    let n = app.loaded_models.len();
    if n == 0 {
        return;
    }

    // Give each loaded model equal vertical space
    let constraints: Vec<Constraint> = app
        .loaded_models
        .iter()
        .map(|_| Constraint::Ratio(1, n as u32))
        .collect();

    let model_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for (i, model_data) in app.loaded_models.iter().enumerate() {
        draw_model_panel(f, app, model_data, model_chunks[i]);
    }
}

fn draw_model_panel(
    f: &mut Frame,
    app: &App,
    model: &crate::api::LoadedModelData,
    area: Rect,
) {
    let active = app.active_slot_count(&model.model_id);
    let total = app.total_slot_count(&model.model_id);
    let tps = app.model_tps(&model.model_id);

    let meta_str = model.meta.as_ref().map(|m| {
        let mut parts = vec![];
        if let Some(p) = m.n_params {
            parts.push(fmt_params(p));
        }
        if let Some(s) = m.size {
            parts.push(fmt_bytes(s));
        }
        if let Some(ctx) = m.n_ctx_train {
            parts.push(format!("ctx:{}", fmt_ctx(ctx)));
        }
        parts.join("  ")
    }).unwrap_or_default();

    let title = format!(
        " {} :{} │ {active}/{total} slots │ {tps:.1} t/s │ {meta_str} ",
        shorten_model_id(&model.model_id),
        model.port,
    );

    // Split area: slots table + sparkline
    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(3)])
        .split(area);

    // ── Slots table ──
    let slot_header = Row::new(vec![
        Cell::from("Slot").style(Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD)),
        Cell::from("State").style(Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD)),
        Cell::from("Task").style(Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD)),
        Cell::from("Generated").style(Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD)),
        Cell::from("t/s").style(Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD)),
    ])
    .height(1)
    .bottom_margin(0);

    let slot_rows: Vec<Row> = model
        .slots
        .iter()
        .map(|slot| {
            let slot_tps = app.slot_tps(&model.model_id, slot.id);
            let in_prefill = app.slot_in_prefill(&model.model_id, slot.id);
            let (state_cell, state_style) = if in_prefill {
                (
                    Cell::from("prefill"),
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                )
            } else if slot.is_processing {
                (
                    Cell::from("generate"),
                    Style::default().fg(ACTIVE_COLOR).add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    Cell::from("idle"),
                    Style::default().fg(DIM_COLOR),
                )
            };

            let task_str = match slot.id_task {
                Some(id) => format!("#{id}"),
                None => String::new(),
            };

            let tps_str = if slot_tps > 0.1 {
                format!("{slot_tps:.1}")
            } else {
                String::new()
            };

            let row_style = if slot.is_processing {
                state_style
            } else {
                Style::default().fg(DIM_COLOR)
            };

            Row::new(vec![
                Cell::from(format!("{}", slot.id)).style(row_style),
                state_cell,
                Cell::from(task_str).style(row_style),
                Cell::from(format!("{}", slot.n_decoded())).style(row_style),
                Cell::from(tps_str).style(
                    if slot.is_processing {
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(DIM_COLOR)
                    }
                ),
            ])
        })
        .collect();

    let slots_table = Table::new(
        slot_rows,
        [
            Constraint::Length(4),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(7),
        ],
    )
    .header(slot_header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(Style::default().fg(LOADED_COLOR).add_modifier(Modifier::BOLD)),
    );

    let mut state = TableState::default();
    f.render_stateful_widget(slots_table, inner_chunks[0], &mut state);

    // ── Throughput sparkline ──
    let tps_history = app.model_tps_history(&model.model_id);
    let max_tps = tps_history.iter().cloned().fold(0.0f64, f64::max).max(1.0);
    let sparkline_data: Vec<u64> = tps_history
        .iter()
        .map(|v| (*v / max_tps * 100.0) as u64)
        .collect();

    let spark_title = format!(" throughput (max {max_tps:.1} t/s) ");
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .title(spark_title)
                .title_style(Style::default().fg(DIM_COLOR)),
        )
        .data(&sparkline_data)
        .style(Style::default().fg(Color::Green));

    f.render_widget(sparkline, inner_chunks[1]);
}

fn draw_footer(f: &mut Frame, area: Rect) {
    let text = Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("/"),
        Span::styled("Esc", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" quit  "),
        Span::styled(" r ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" force refresh  "),
        Span::styled(" ↑↓ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" scroll  "),
        Span::raw("  refreshes every 2s"),
    ]);

    let para = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Left);
    f.render_widget(para, area);
}

/// Shorten long HuggingFace-style model IDs for display
fn shorten_model_id(id: &str) -> String {
    // org/repo:quant  →  repo:quant  (keep last 30 chars if still long)
    let s = if let Some(slash) = id.rfind('/') {
        &id[slash + 1..]
    } else {
        id
    };
    if s.len() > 32 {
        format!("…{}", &s[s.len() - 31..])
    } else {
        s.to_string()
    }
}

fn fmt_ctx(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{n}")
    }
}
