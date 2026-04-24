mod api;
mod app;
mod ui;

use anyhow::{Result, bail};
use app::App;
 use crossterm::{
       event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
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

fn parse_args() -> Result<(Duration, String, String)> {
    let mut args = std::env::args().skip(1);
    let mut interval = Duration::from_secs(1);
    let mut server_url: Option<String> = None;
    let mut api_key: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                eprintln!("Usage: llama-monitor [OPTIONS] [INTERVAL_SECS]");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  --url <URL>      Router URL (default: http://localhost:8080)");
                eprintln!("  --key <KEY>      API key (default: KEY-SECRET)");
                eprintln!();
                eprintln!("Environment variables:");
                eprintln!("  LLM_DEFAULT_URL   Router URL (overridden by --url)");
                eprintln!("  LLM_DEFAULT_KEY   API key (overridden by --key)");
                eprintln!();
                eprintln!("  INTERVAL_SECS  Refresh interval in seconds (default: 1)");
                std::process::exit(0);
            }
            "--url" => {
                if let Some(val) = args.next() {
                    server_url = Some(val);
                } else {
                    bail!("--url requires a value");
                }
            }
            "--key" => {
                if let Some(val) = args.next() {
                    api_key = Some(val);
                } else {
                    bail!("--key requires a value");
                }
            }
            s => {
                if let Ok(secs) = s.parse::<f64>() {
                    if secs > 0.0 {
                        interval = Duration::from_secs_f64(secs);
                    } else {
                        bail!("Invalid interval {:?}: expected a positive number of seconds", s);
                    }
                } else {
                    bail!("Unknown argument {:?}", s);
                }
            }
        }
    }

    let server_url = server_url
        .or_else(|| std::env::var("LLM_DEFAULT_URL").ok())
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    let api_key = api_key
        .or_else(|| std::env::var("LLM_DEFAULT_KEY").ok())
        .unwrap_or_else(|| "KEY-SECRET".to_string());

    Ok((interval, server_url, api_key))
}

#[tokio::main]
async fn main() -> Result<()> {
    let (tick_rate, server_url, api_key) = parse_args()?;

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, tick_rate, server_url, api_key).await;

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
