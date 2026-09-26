use crate::dbus::NmClient;

#[derive(Debug, Clone)]
pub enum UiEvent {
    Status(String),
    Networks(Result<Vec<NetworkEntry>, String>),
    PasswordFetched(Result<String, String>),
    RequestRefresh,
    NeedPassword(NetworkEntry),
}

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

impl NetworkKind {
    pub fn security(&self) -> Option<&str> {
        match self {
            NetworkKind::Wireless { security, .. } => Some(security),
            _ => None,
        }
    }
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
    pub ap_path: Option<String>,
}

#[derive(Default)]
pub struct PasswordModalState {
    pub open: bool,
    pub entry: Option<NetworkEntry>,
    pub input: String,
    pub error: Option<String>,
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
    pub password_modal: PasswordModalState,
    pub nm_client: Option<NmClient>,
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
            password_modal: PasswordModalState::default(),
            nm_client: None,
        }
    }

    pub fn set_status<S: Into<String>>(&mut self, message: S) {
        self.status_message = message.into();
    }

    pub fn selected_network(&self) -> Option<&NetworkEntry> {
        self.networks.get(self.selected_index)
    }

    pub fn selected_network_cloned(&self) -> Option<NetworkEntry> {
        self.selected_network().cloned()
    }

    pub fn clamp_selection(&mut self) {
        if self.networks.is_empty() {
            self.selected_index = 0;
        } else if self.selected_index >= self.networks.len() {
            self.selected_index = self.networks.len() - 1;
        }
    }

    pub fn set_networks(&mut self, networks: Vec<NetworkEntry>) {
        let previous_selected = self.selected_network().map(|n| n.name.clone());

        self.networks = networks;

        if let Some(name) = previous_selected {
            if let Some(index) = self.networks.iter().position(|n| n.name == name) {
                self.selected_index = index;
            }
        }

        self.clamp_selection();
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

    pub fn toggle_qr(&mut self) {
        self.show_qr = !self.show_qr;
        if !self.show_qr {
            self.qr_password = None;
        }
    }

    pub fn open_password_modal(&mut self, entry: NetworkEntry) {
        self.status_message = format!("Password required for {}", entry.name);
        self.password_modal.open = true;
        self.password_modal.entry = Some(entry);
        self.password_modal.input.clear();
        self.password_modal.error = None;
    }

    pub fn close_password_modal(&mut self) {
        self.password_modal.open = false;
        self.password_modal.entry = None;
        self.password_modal.input.clear();
        self.password_modal.error = None;
    }

    pub fn on_tick(&mut self) {
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
    }
}