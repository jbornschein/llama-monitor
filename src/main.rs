mod api;
mod app;
mod ui;

use anyhow::{Result, bail};
use app::App;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use reqwest::header::HeaderMap;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

fn parse_args() -> Result<Duration> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => Ok(Duration::from_secs(1)),
        Some("-h" | "--help") => {
            eprintln!("Usage: llama-monitor [INTERVAL_SECS]");
            eprintln!("  INTERVAL_SECS  Refresh interval in seconds (default: 1)");
            std::process::exit(0);
        }
        Some(s) => match s.parse::<f64>() {
            Ok(secs) if secs > 0.0 => Ok(Duration::from_secs_f64(secs)),
            _ => bail!("Invalid interval {:?}: expected a positive number of seconds", s),
        },
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let tick_rate = parse_args()?;

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, tick_rate).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, tick_rate: Duration) -> Result<()> {
    let server_url = "http://localhost:8080".to_string();
    let mut headers = HeaderMap::new();
    headers.insert("Authorization", "Bearer KEY-SECRET".parse().unwrap());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .default_headers(headers)
        .build()?;

    let mut app = App::new(server_url.clone());

    // Channel for background fetch results
    let (tx, mut rx) = mpsc::channel(16);

    // Initial fetch
    {
        let client = client.clone();
        let url = server_url.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let data = api::fetch_all(&client, &url).await;
            let _ = tx.send(data).await;
        });
    }

    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        // Poll for events with short timeout so we stay responsive
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_default();

        if event::poll(timeout.min(Duration::from_millis(100)))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Char('r') => {
                            // Force refresh
                            app.set_refreshing(true);
                            let client = client.clone();
                            let url = server_url.clone();
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let data = api::fetch_all(&client, &url).await;
                                let _ = tx.send(data).await;
                            });
                            last_tick = Instant::now();
                        }
                        KeyCode::Up => app.scroll_up(),
                        KeyCode::Down => app.scroll_down(),
                        _ => {}
                    }
                }
            }
        }

        // Check for new data
        while let Ok(result) = rx.try_recv() {
            app.update(result);
        }

        // Periodic refresh
        if last_tick.elapsed() >= tick_rate {
            app.set_refreshing(true);
            let client = client.clone();
            let url = server_url.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let data = api::fetch_all(&client, &url).await;
                let _ = tx.send(data).await;
            });
            last_tick = Instant::now();
        }
    }

    Ok(())
}
