mod app;
mod dbus;
mod qr;
mod ui;

use app::{App, NetworkEntry, NetworkKind, UiEvent, ViewMode, SpeedTestUpdateData, SpeedPhase};
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
    sync::mpsc::{channel, Sender},
    thread,
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

// --- REAL SPEED TEST LOGIC WITH UREQ ---

fn run_ping_test(tx: Sender<UiEvent>) {
    let targets = [
        "https://www.google.com/generate_204",
        "https://cloudflare.com/cdn-cgi/trace",
        "https://speed.cloudflare.com/__down?bytes=0",
    ];
    let mut latencies = Vec::new();
    
    let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
        phase: SpeedPhase::Ping,
        value: None,
        progress: 0,
        done: false,
    }));

    for i in 0..5 {
        let url = targets[i % targets.len()];
        let start = Instant::now();
        
        // Perform a HEAD request or simple GET to measure TTFB
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .build();
            
        match agent.head(url).call() {
            Ok(_resp) => {
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                latencies.push(elapsed);
                
                let progress = ((i + 1) * 100) / 5;
                let avg_so_far = latencies.iter().sum::<f64>() / latencies.len() as f64;
                let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
                    phase: SpeedPhase::Ping,
                    value: Some(avg_so_far),
                    progress: progress as u16,
                    done: false,
                }));
            }
            Err(_) => {
                // Skip failures silently for robustness
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    let final_avg = if latencies.is_empty() {
        999.0
    } else {
        latencies.iter().sum::<f64>() / latencies.len() as f64
    };

    let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
        phase: SpeedPhase::Ping,
        value: Some(final_avg),
        progress: 100,
        done: true,
    }));
}

fn run_download_test(tx: Sender<UiEvent>) {
    // Reliable test file endpoint
    let url = "https://speed.cloudflare.com/__down?bytes=10000000"; // 10MB
    
    let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
        phase: SpeedPhase::Download,
        value: None,
        progress: 0,
        done: false,
    }));

    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(30))
        .timeout_write(Duration::from_secs(5))
        .build();

    match agent.get(url).call() {
        Ok(resp) => { // Removed 'mut' from resp binding
            let start_time = Instant::now();
            let mut total_bytes = 0usize;
            let mut buf = [0u8; 4096];
            let mut last_update_time = Instant::now();
            
            // Estimate expected size roughly for progress bar
            let expected_size = 10_000_000.0; 

            use std::io::Read;
            // FIX: Declare reader as mutable
            let mut reader = resp.into_reader(); 
            
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        total_bytes += n;
                        
                        if last_update_time.elapsed() > Duration::from_millis(200) {
                            let elapsed = start_time.elapsed().as_secs_f64();
                            if elapsed > 0.0 {
                                let mbps = (total_bytes as f64 * 8.0) / (elapsed * 1_000_000.0);
                                let pct = ((total_bytes as f64 / expected_size) * 100.0) as u16;
                                
                                let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
                                    phase: SpeedPhase::Download,
                                    value: Some(mbps),
                                    progress: pct.min(99),
                                    done: false,
                                }));
                            }
                            last_update_time = Instant::now();
                        }
                    }
                    Err(_) => break,
                }
            }

            let elapsed = start_time.elapsed().as_secs_f64();
            let final_mbps = if elapsed > 0.0 {
                (total_bytes as f64 * 8.0) / (elapsed * 1_000_000.0)
            } else {
                0.0
            };

            let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
                phase: SpeedPhase::Download,
                value: Some(final_mbps),
                progress: 100,
                done: true,
            }));
        }
        Err(_) => {
             let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
                phase: SpeedPhase::Download,
                value: Some(0.0),
                progress: 100,
                done: true,
            }));
        }
    }
}

fn run_upload_test(tx: Sender<UiEvent>) {
    // Cloudflare accepts POST bodies and discards them efficiently
    let url = "https://speed.cloudflare.com/__up";
    let payload_size = 5 * 1024 * 1024; // 5 MB
    let payload = vec![0u8; payload_size];
    
    let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
        phase: SpeedPhase::Upload,
        value: None,
        progress: 0,
        done: false,
    }));

    // FIX: Use .timeout() instead of .timeout_send()
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(30)) 
        .build();

    let start_time = Instant::now();
    
    // Splitting into chunks for better UX feedback during upload
    let chunk_count = 5;
    let chunk_size = payload_size / chunk_count;
    let mut total_sent = 0;
    
    for i in 0..chunk_count {
        let start_idx = i * chunk_size;
        let end_idx = if i == chunk_count - 1 { payload_size } else { start_idx + chunk_size };
        let chunk = &payload[start_idx..end_idx];
        
        match agent.post(url).send_bytes(chunk) {
            Ok(_) => {
                total_sent += chunk.len();
                let elapsed = start_time.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    let mbps = (total_sent as f64 * 8.0) / (elapsed * 1_000_000.0);
                    let pct = ((total_sent as f64 / payload_size as f64) * 100.0) as u16;
                    
                    let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
                        phase: SpeedPhase::Upload,
                        value: Some(mbps),
                        progress: pct,
                        done: false,
                    }));
                }
            }
            Err(_) => {
                // Abort on failure
                break;
            }
        }
    }

    let elapsed = start_time.elapsed().as_secs_f64();
    let final_mbps = if elapsed > 0.0 {
        (total_sent as f64 * 8.0) / (elapsed * 1_000_000.0)
    } else {
        0.0
    };

    let _ = tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
        phase: SpeedPhase::Upload,
        value: Some(final_mbps),
        progress: 100,
        done: true,
    }));
}

fn spawn_speed_test(tx: UnboundedSender<UiEvent>) {
    let (internal_tx, internal_rx) = channel::<UiEvent>();
    
    // Forwarder thread to bridge std::mpsc to tokio::mpsc
    let forward_tx = tx.clone();
    thread::spawn(move || {
        while let Ok(event) = internal_rx.recv() {
            let _ = forward_tx.send(event);
        }
    });

    // Worker thread executing blocking network tasks
    let tx_clone = internal_tx.clone();
    thread::spawn(move || {
        run_ping_test(tx_clone.clone());
        run_download_test(tx_clone.clone());
        run_upload_test(tx_clone);
        
        let _ = internal_tx.send(UiEvent::SpeedTestUpdate(SpeedTestUpdateData {
            phase: SpeedPhase::Complete,
            value: None,
            progress: 100,
            done: true,
        }));
    });
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
                UiEvent::SpeedTestUpdate(update) => {
                    app.apply_speed_update(update);
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
                                    app.speed_test.is_running = false;
                                    app.set_status("Returned to networks");
                                } else {
                                    app.should_quit = true;
                                }
                            }
                            KeyCode::Char('n') => {
                                if app.view_mode == ViewMode::Normal {
                                     app.toggle_speed_test();
                                     if app.view_mode == ViewMode::SpeedTest {
                                         spawn_speed_test(tx.clone());
                                     }
                                } else {
                                    app.toggle_speed_test();
                                }
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
                                    // Restart speed test
                                    app.start_speed_test();
                                    spawn_speed_test(tx.clone());
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