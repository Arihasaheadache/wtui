mod app;
mod dbus;
mod qr;
mod ui;

use app::{App, ViewMode};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{error::Error, io, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    if let Ok(nm) = dbus::NmClient::new().await {
        app.refresh_networks(&nm).await;
    }

    let tick_rate = Duration::from_millis(100); // 100ms provides fluid cloud movement
    let mut last_tick = std::time::Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => {
                            if app.view_mode == ViewMode::SpeedTest {
                                app.view_mode = ViewMode::Normal;
                            } else {
                                break;
                            }
                        }
                        KeyCode::Esc => {
                            if app.view_mode == ViewMode::SpeedTest {
                                app.view_mode = ViewMode::Normal;
                            } else {
                                break;
                            }
                        }
                        KeyCode::Char('n') => {
                            app.toggle_speed_test();
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            if app.view_mode == ViewMode::Normal {
                                app.next_network();
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if app.view_mode == ViewMode::Normal {
                                app.previous_network();
                            }
                        }
                        KeyCode::Char('s') => {
                            if app.view_mode == ViewMode::Normal {
                                app.toggle_qr().await;
                            }
                        }
                        KeyCode::Char('a') => {
                            if app.view_mode == ViewMode::Normal {
                                app.toggle_autoconnect().await;
                            }
                        }
                        KeyCode::Char('c') => {
                            if app.view_mode == ViewMode::Normal {
                                app.toggle_connect().await;
                            }
                        }
                        KeyCode::Char('r') => {
                            if app.view_mode == ViewMode::Normal {
                                app.rescan().await;
                            } else {
                                app.start_speed_test();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.on_tick().await;
            last_tick = std::time::Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
