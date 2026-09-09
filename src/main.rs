mod app;
mod dbus;
mod qr;
mod ui;

use app::App;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{error::Error, io, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 1. Setup terminal in raw mode and enter alternate screen
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 2. Initialize application state
    let mut app = App::new();

    // 3. Connect to NetworkManager via D-Bus and perform initial fetch
    if let Ok(nm) = dbus::NmClient::new().await {
        app.refresh_networks(&nm).await;
    }

    // 4. Main event & render loop
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = std::time::Instant::now();

    loop {
        // Draw the frame
        terminal.draw(|f| ui::draw(f, &mut app))?;

        // Calculate remaining timeout until the next tick
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        // Poll for input events
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Only handle Press events (ignore KeyRelease events)
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            break;
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.next_network();
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.previous_network();
                        }
                        KeyCode::Char('s') => {
                            app.toggle_qr().await;
                        }
                        KeyCode::Char('a') => {
                            app.toggle_autoconnect().await;
                        }
                        KeyCode::Char('c') => {
                            app.toggle_connect().await;
                        }
                        KeyCode::Char('r') => {
                            // Rescan networks
                            app.rescan().await;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Periodic background tick
        if last_tick.elapsed() >= tick_rate {
            app.on_tick().await;
            last_tick = std::time::Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    // 5. Restore terminal to normal mode cleanly
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
