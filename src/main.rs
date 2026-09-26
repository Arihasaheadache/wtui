mod app;
mod dbus;
mod qr;
mod ui;

use app::{App, NetworkEntry, NetworkKind, UiEvent, ViewMode};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dbus::NmClient;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    error::Error,
    io,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{self, UnboundedSender};

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

fn spawn_refresh(client: NmClient, tx: UnboundedSender<UiEvent>, announce: bool) {
    tokio::spawn(async move {
        let result = client
            .fetch_all_networks()
            .await
            .map_err(|e| e.to_string());

        let ok = result.is_ok();
        let _ = tx.send(UiEvent::Networks(result));

        if announce && ok {
            let _ = tx.send(UiEvent::Status("Scan complete".to_string()));
        }
    });
}

fn spawn_connect(
    client: NmClient,
    tx: UnboundedSender<UiEvent>,
    entry: NetworkEntry,
    password: Option<String>,
) {
    let name = entry.name.clone();
    let entry_for_password_prompt = entry.clone();

    tokio::spawn(async move {
        let _ = tx.send(UiEvent::Status(format!("Connecting to {name}...")));

        // FIX: Convert Box<dyn Error> to String immediately so it doesn't cross the await boundary
        let result = client
            .connect_or_add_network(&entry, password.as_deref())
            .await
            .map_err(|e| e.to_string());

        match result {
            Err(message) => {
                let lower = message.to_lowercase();

                if password.is_none()
                    && (lower.contains("secret")
                        || lower.contains("psk")
                        || lower.contains("802-11-wireless-security")
                        || lower.contains("no secrets"))
                {
                    let _ = tx.send(UiEvent::Status(format!("{name} requires a password")));
                    let _ = tx.send(UiEvent::NeedPassword(entry_for_password_prompt));
                } else {
                    let _ = tx.send(UiEvent::Status(format!("Connection failed: {message}")));
                }
            }
            Ok(_) => {
                let _ = tx.send(UiEvent::Status(format!("Waiting for {name} to activate...")));

                // FIX: Map error to string here as well
                let wait_result = client
                    .wait_for_connected(&name, Duration::from_secs(20))
                    .await
                    .map_err(|e| e.to_string());

                match wait_result {
                    Ok(_) => {
                        let _ = tx.send(UiEvent::Status(format!("Connected to {name}")));
                    }
                    Err(e) => {
                        let _ = tx.send(UiEvent::Status(format!(
                            "Connection request sent, but not active yet: {e}"
                        )));
                    }
                }
            }
        }

        let _ = tx.send(UiEvent::RequestRefresh);
    });
}

fn spawn_disconnect(client: NmClient, tx: UnboundedSender<UiEvent>, entry: NetworkEntry) {
    let name = entry.name.clone();
    let device_path = entry.dbus_path.clone();

    tokio::spawn(async move {
        let _ = tx.send(UiEvent::Status(format!("Disconnecting from {name}...")));

        // FIX: Convert Box<dyn Error> to String immediately
        let result = client
            .disconnect_network(&device_path)
            .await
            .map_err(|e| e.to_string());

        match result {
            Err(msg) => {
                let _ = tx.send(UiEvent::Status(format!("Disconnect failed: {msg}")));
            }
            Ok(_) => {
                let _ = tx.send(UiEvent::Status(format!("Waiting for {name} to disconnect...")));

                // FIX: Map error to string here as well
                let wait_result = client
                    .wait_for_disconnected(&name, Duration::from_secs(10))
                    .await
                    .map_err(|e| e.to_string());

                match wait_result {
                    Ok(_) => {
                        let _ = tx.send(UiEvent::Status(format!("Disconnected from {name}")));
                    }
                    Err(msg) => {
                        let _ = tx.send(UiEvent::Status(format!(
                            "Disconnect requested, but not confirmed yet: {msg}"
                        )));
                    }
                }
            }
        }

        let _ = tx.send(UiEvent::RequestRefresh);
    });
}

fn invalid_wpa_password(security: &str, password: &str) -> bool {
    if security.contains("Open")
        || security.contains("Enterprise")
        || security.contains("SAE")
        || security.contains("WEP")
    {
        return false;
    }
    if password.len() == 64 {
        return false;
    }
    password.len() < 8 || password.len() > 63
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        original_hook(panic_info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    let client = match NmClient::new().await {
        Ok(client) => client,
        Err(e) => {
            restore_terminal();
            return Err(e);
        }
    };

    app.set_status("Loading networks...");

    match client.fetch_all_networks().await {
        Ok(networks) => {
            app.set_networks(networks);
            app.set_status("Ready");
        }
        Err(e) => {
            app.set_status(format!("D-Bus error: {e}"));
        }
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<UiEvent>();

    let tick_rate = Duration::from_millis(100);
    let refresh_interval = Duration::from_secs(10);

    let mut last_tick = Instant::now();
    let mut last_refresh = Instant::now();
    let mut refresh_pending = false;

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        if app.should_quit {
            break;
        }

        while let Ok(event) = rx.try_recv() {
            match event {
                UiEvent::Status(message) => {
                    app.set_status(message);
                }
                UiEvent::Networks(result) => {
                    refresh_pending = false;
                    last_refresh = Instant::now();

                    match result {
                        Ok(networks) => {
                            app.set_networks(networks);
                        }
                        Err(e) => {
                            app.set_status(format!("Refresh failed: {e}"));
                        }
                    }
                }
                UiEvent::PasswordFetched(result) => match result {
                    Ok(password) => {
                        app.qr_password = Some(password);
                        app.set_status("Password loaded for QR code");
                    }
                    Err(e) => {
                        app.qr_password = Some(String::new());
                        app.set_status(format!("Password error: {e}"));
                    }
                },
                UiEvent::RequestRefresh => {
                    spawn_refresh(client.clone(), tx.clone(), false);
                }
                UiEvent::NeedPassword(entry) => {
                    app.open_password_modal(entry);
                }
            }
        }

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if app.password_modal.open {
                        match key.code {
                            KeyCode::Esc => {
                                app.close_password_modal();
                            }
                            KeyCode::Enter => {
                                if let Some(entry) = app.password_modal.entry.clone() {
                                    let password = app.password_modal.input.clone();

                                    if password.is_empty() {
                                        app.password_modal.error =
                                            Some("Password cannot be empty".to_string());
                                    } else if let NetworkKind::Wireless { security, .. } = &entry.kind
                                    {
                                        if invalid_wpa_password(security, &password) {
                                            app.password_modal.error = Some(
                                                "WPA/WPA2 password must be 8-63 characters"
                                                    .to_string(),
                                            );
                                        } else {
                                            app.close_password_modal();
                                            spawn_connect(
                                                client.clone(),
                                                tx.clone(),
                                                entry,
                                                Some(password),
                                            );
                                        }
                                    }
                                }
                            }
                            KeyCode::Backspace => {
                                app.password_modal.input.pop();
                            }
                            KeyCode::Char(c) => {
                                app.password_modal.input.push(c);
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                if app.view_mode == ViewMode::SpeedTest {
                                    app.view_mode = ViewMode::Normal;
                                    app.set_status("Returned to networks");
                                } else {
                                    app.should_quit = true;
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
                                    let turning_on = !app.show_qr;
                                    app.toggle_qr();

                                    if turning_on {
                                        if let Some(entry) = app.selected_network_cloned() {
                                            if let Some(conn_path) = entry.connection_path.clone() {
                                                app.set_status("Fetching password for QR code...");

                                                let client_clone = client.clone();
                                                let tx_clone = tx.clone();

                                                tokio::spawn(async move {
                                                    let result = client_clone
                                                        .get_wifi_password(&conn_path)
                                                        .await
                                                        .map_err(|e| e.to_string());

                                                    let _ = tx_clone
                                                        .send(UiEvent::PasswordFetched(result));
                                                });
                                            } else {
                                                app.qr_password = Some(String::new());
                                                app.set_status(
                                                    "No saved profile; cannot fetch password",
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('a') => {
                                if app.view_mode == ViewMode::Normal {
                                    if let Some(entry) = app.selected_network_cloned() {
                                        if let Some(conn_path) = entry.connection_path.clone() {
                                            let new_state = !entry.autoconnect;
                                            app.set_status(format!(
                                                "Setting autoconnect to {}...",
                                                new_state
                                            ));

                                            let client_clone = client.clone();
                                            let tx_clone = tx.clone();

                                            tokio::spawn(async move {
                                                let result = client_clone
                                                    .set_autoconnect(&conn_path, new_state)
                                                    .await;

                                                let message = match result {
                                                    Ok(_) => format!("Autoconnect set to {new_state}"),
                                                    Err(e) => format!("Autoconnect failed: {e}"),
                                                };

                                                let _ = tx_clone.send(UiEvent::Status(message));
                                                let _ = tx_clone.send(UiEvent::RequestRefresh);
                                            });
                                        } else {
                                            app.set_status(
                                                "No saved connection profile to set autoconnect",
                                            );
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('c') | KeyCode::Enter => {
                                if app.view_mode == ViewMode::Normal {
                                    if let Some(entry) = app.selected_network_cloned() {
                                        if entry.is_connected {
                                            spawn_disconnect(client.clone(), tx.clone(), entry);
                                        } else {
                                            match &entry.kind {
                                                NetworkKind::Wireless { security, .. }
                                                    if security != "Open"
                                                        && !security.contains("Enterprise")
                                                        && entry.connection_path.is_none() =>
                                                {
                                                    app.open_password_modal(entry.clone());
                                                }
                                                _ => {
                                                    spawn_connect(
                                                        client.clone(),
                                                        tx.clone(),
                                                        entry,
                                                        None,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('r') => {
                                if app.view_mode == ViewMode::Normal {
                                    app.set_status("Scanning for networks...");
                                    refresh_pending = true;
                                    spawn_refresh(client.clone(), tx.clone(), true);
                                } else {
                                    app.start_speed_test();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.on_tick();
            last_tick = Instant::now();
        }

        if !refresh_pending
            && !app.password_modal.open
            && last_refresh.elapsed() >= refresh_interval
        {
            refresh_pending = true;
            spawn_refresh(client.clone(), tx.clone(), false);
        }

        if app.should_quit {
            break;
        }
    }

    restore_terminal();
    terminal.show_cursor()?;

    Ok(())
}