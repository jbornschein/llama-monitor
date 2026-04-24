mod api;
mod app;
mod ui;

use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use reqwest::header::HeaderMap;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, time::{Duration, Instant}};
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(name = "llama-monitor", about = "Terminal UI for monitoring a llama.cpp router server")]
pub struct MonitorArgs {
    /// router server url
    #[arg(long, short = 'u', default_value = "http://localhost:8080", env = "LLM_DEFAULT_URL")]
    pub url: String,

    /// api key for authentication
    #[arg(short = 'k', long, default_value = "", env = "LLM_DEFAULT_KEY")]
    pub key: String,

    /// refresh interval in seconds
    #[arg(long, short = 'i', default_value = "1")]
    pub interval: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = MonitorArgs::parse();

    if args.interval <= 0.0 {
        anyhow::bail!("Invalid interval: expected a positive number of seconds");
    }
    let tick_rate = Duration::from_secs_f64(args.interval);

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, tick_rate, args.url, args.key).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
}

async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, tick_rate: Duration, server_url: String, api_key: String) -> Result<()> {
    let mut headers = HeaderMap::new();
    let auth_header = format!("Bearer {}", api_key).parse().map_err(|e| anyhow::anyhow!("Invalid auth header: {e}"))?;
    headers.insert("Authorization", auth_header);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .default_headers(headers)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e}"))?;

    let mut app = App::new(server_url.clone());

    // Channel for background fetch results (unbounded to prevent blocking)
    let (tx, mut rx) = mpsc::unbounded_channel();

    // Initial fetch
    {
        let client = client.clone();
        let url = server_url.clone();
        let key = api_key.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let data = api::fetch_all(&client, &url, &key).await;
            let _ = tx.send(data);
        });
    }

    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        // Poll for events with short timeout so we stay responsive
        let elapsed = last_tick.elapsed();
        let timeout = if elapsed < tick_rate {
            tick_rate - elapsed
        } else {
            Duration::ZERO
        };

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
                            let key = api_key.clone();
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let data = api::fetch_all(&client, &url, &key).await;
                                let _ = tx.send(data);
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
            let key = api_key.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let data = api::fetch_all(&client, &url, &key).await;
                let _ = tx.send(data);
            });
            last_tick = Instant::now();
        }
    }

    Ok(())
}
