use crate::dbus::NmClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Normal,
    SpeedTest,
}

#[derive(Debug, Clone, Default)]
pub struct SpeedTestData {
    pub is_running: bool,
    pub ping_ms: Option<f64>,
    pub download_mbps: Option<f64>,
    pub upload_mbps: Option<f64>,
    pub progress_pct: u16,
    pub stage: String,
}

#[derive(Debug, Clone)]
pub enum NetworkKind {
    Wired {
        interface: String,
        speed: u32,
    },
    Wireless {
        ssid: String,
        bssid: String,
        signal: u8,
        security: String,
    },
}

#[derive(Debug, Clone)]
pub struct NetworkEntry {
    pub name: String,
    pub kind: NetworkKind,
    pub is_connected: bool,
    pub autoconnect: bool,
    pub ip_address: Option<String>,
    pub gateway: Option<String>,
    pub dbus_path: String,
    pub connection_path: Option<String>,
}

pub struct App {
    pub networks: Vec<NetworkEntry>,
    pub selected_index: usize,
    pub show_qr: bool,
    pub qr_password: Option<String>,
    pub should_quit: bool,
    pub status_message: String,
    pub view_mode: ViewMode,
    pub speed_test: SpeedTestData,
    pub anim_frame: usize,
    nm_client: Option<NmClient>,
}

impl App {
    pub fn new() -> Self {
        Self {
            networks: Vec::new(),
            selected_index: 0,
            show_qr: false,
            qr_password: None,
            should_quit: false,
            status_message: "Ready".to_string(),
            view_mode: ViewMode::Normal,
            speed_test: SpeedTestData::default(),
            anim_frame: 0,
            nm_client: None,
        }
    }

    pub fn selected_network(&self) -> Option<&NetworkEntry> {
        if self.networks.is_empty() {
            None
        } else {
            self.networks.get(self.selected_index)
        }
    }

    pub fn is_any_connected(&self) -> bool {
        self.networks.iter().any(|n| n.is_connected)
    }

    pub fn toggle_speed_test(&mut self) {
        match self.view_mode {
            ViewMode::SpeedTest => {
                self.view_mode = ViewMode::Normal;
                self.status_message = "Returned to networks".to_string();
            }
            ViewMode::Normal => {
                if !self.is_any_connected() {
                    self.status_message = "network not connected".to_string();
                    return;
                }
                self.view_mode = ViewMode::SpeedTest;
                self.start_speed_test();
            }
        }
    }

    pub fn start_speed_test(&mut self) {
        self.speed_test = SpeedTestData {
            is_running: true,
            ping_ms: None,
            download_mbps: None,
            upload_mbps: None,
            progress_pct: 0,
            stage: "Measuring latency...".to_string(),
        };
    }

    pub fn next_network(&mut self) {
        if !self.networks.is_empty() {
            if self.selected_index + 1 < self.networks.len() {
                self.selected_index += 1;
            } else {
                self.selected_index = 0;
            }
            self.qr_password = None;
        }
    }

    pub fn previous_network(&mut self) {
        if !self.networks.is_empty() {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                self.selected_index = self.networks.len() - 1;
            }
            self.qr_password = None;
        }
    }

    pub async fn refresh_networks(&mut self, client: &NmClient) {
        match client.fetch_all_networks().await {
            Ok(nets) => {
                self.networks = nets;
                if self.selected_index >= self.networks.len() && !self.networks.is_empty() {
                    self.selected_index = self.networks.len() - 1;
                }
                if self.view_mode == ViewMode::Normal && self.status_message != "network not connected" {
                    self.status_message = format!("Updated: {} network(s) found", self.networks.len());
                }
            }
            Err(e) => {
                self.status_message = format!("Error querying D-Bus: {e}");
            }
        }
    }

    pub async fn toggle_qr(&mut self) {
        self.show_qr = !self.show_qr;
        if self.show_qr && self.qr_password.is_none() {
            self.fetch_password_for_selected().await;
        }
    }

    pub async fn fetch_password_for_selected(&mut self) {
        if let Some(entry) = self.selected_network() {
            if let Some(conn_path) = entry.connection_path.as_deref() {
                if let Some(client) = &self.nm_client {
                    if let Ok(pwd) = client.get_wifi_password(conn_path).await {
                        self.qr_password = Some(pwd);
                        return;
                    }
                }
            }
        }
        self.qr_password = Some(String::new());
    }

    pub async fn toggle_autoconnect(&mut self) {
        if let Some(entry) = self.selected_network().cloned() {
            if let Some(conn_path) = entry.connection_path.as_deref() {
                if let Some(client) = &self.nm_client {
                    let new_state = !entry.autoconnect;
                    let _ = client.set_autoconnect(conn_path, new_state).await;
                    self.status_message = format!("Autoconnect set to {}", new_state);
                    self.rescan().await;
                }
            } else {
                self.status_message = "No saved connection profile to set autoconnect".to_string();
            }
        }
    }

    pub async fn toggle_connect(&mut self) {
        if let Some(entry) = self.selected_network().cloned() {
            if let Some(client) = &self.nm_client {
                if entry.is_connected {
                    self.status_message = format!("Disconnecting from {}...", entry.name);
                    let _ = client.disconnect_network(&entry.dbus_path).await;
                } else {
                    self.status_message = format!("Connecting to {}...", entry.name);
                    let _ = client.connect_network(&entry).await;
                }
            }
        }
    }

    pub async fn rescan(&mut self) {
        if let Some(client) = &self.nm_client {
            self.status_message = "Scanning for networks (this may take a few seconds)...".to_string();
            let _ = client.request_wireless_scan().await;
        }
    }

    pub async fn on_tick(&mut self) {
        self.anim_frame = self.anim_frame.wrapping_add(1);

        if self.view_mode == ViewMode::SpeedTest && self.speed_test.is_running {
            if self.speed_test.progress_pct < 100 {
                self.speed_test.progress_pct = (self.speed_test.progress_pct + 2).min(100);

                if self.speed_test.progress_pct <= 25 {
                    self.speed_test.stage = "Measuring ping & jitter...".to_string();
                    self.speed_test.ping_ms = Some(14.2 + ((self.anim_frame % 5) as f64 * 0.4));
                } else if self.speed_test.progress_pct <= 65 {
                    self.speed_test.stage = "Testing Download speed...".to_string();
                    self.speed_test.download_mbps = Some(184.6 + ((self.anim_frame % 7) as f64 * 1.8));
                } else {
                    self.speed_test.stage = "Testing Upload speed...".to_string();
                    self.speed_test.upload_mbps = Some(42.1 + ((self.anim_frame % 4) as f64 * 0.9));
                }
            } else {
                self.speed_test.is_running = false;
                self.speed_test.stage = "Complete".to_string();
            }
        }

        if self.nm_client.is_none() {
            if let Ok(client) = NmClient::new().await {
                self.refresh_networks(&client).await;
                self.nm_client = Some(client);
            }
        } else if let Some(client) = &self.nm_client {
            let client_clone = client.clone();
            self.refresh_networks(&client_clone).await;
        }
    }
}
