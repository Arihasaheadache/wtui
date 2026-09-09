use crate::app::{NetworkEntry, NetworkKind};
use std::collections::HashMap;
use std::error::Error;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

const NM_SERVICE: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_IFACE: &str = "org.freedesktop.NetworkManager";
const NM_SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const NM_SETTINGS_CONN_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const NM_DEV_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const NM_WIRELESS_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const NM_WIRED_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wired";
const NM_AP_IFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
const NM_IP4_IFACE: &str = "org.freedesktop.NetworkManager.IP4Config";

// NetworkManager Device types
const NM_DEVICE_TYPE_ETHERNET: u32 = 1;
const NM_DEVICE_TYPE_WIFI: u32 = 2;

// NM 802-11 Security Flags
const NM_802_11_AP_SEC_NONE: u32 = 0x0;
const NM_802_11_AP_SEC_KEY_MGMT_802_1X: u32 = 0x200;
const NM_802_11_AP_SEC_KEY_MGMT_SAE: u32 = 0x400;

#[derive(Clone)]
pub struct NmClient {
    conn: Connection,
}

impl NmClient {
    pub async fn new() -> Result<Self, Box<dyn Error>> {
        let conn = Connection::system().await?;
        Ok(Self { conn })
    }

    /// Fetch all wired and wireless network endpoints
    pub async fn fetch_all_networks(&self) -> Result<Vec<NetworkEntry>, Box<dyn Error>> {
        let mut entries = Vec::new();

        // 1. Get all saved connection profiles
        let saved_conns = self.get_saved_connections().await.unwrap_or_default();

        // 2. Query NM root proxy for devices
        let nm_proxy = zbus::Proxy::new(&self.conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;
        let devices: Vec<OwnedObjectPath> = nm_proxy.call("GetDevices", &()).await?;

        // 3. Inspect each device
        for dev_path in devices {
            let dev_proxy =
                zbus::Proxy::new(&self.conn, NM_SERVICE, dev_path.as_str(), NM_DEV_IFACE).await?;
            let dev_type: u32 = dev_proxy.get_property("DeviceType").await.unwrap_or(0);
            let dev_state: u32 = dev_proxy.get_property("State").await.unwrap_or(0);
            let iface_name: String = dev_proxy.get_property("Interface").await.unwrap_or_default();

            // DeviceState 100 = NM_DEVICE_STATE_ACTIVATED
            let is_dev_activated = dev_state == 100;

            let (ip_address, gateway) = if is_dev_activated {
                self.get_ip_info(&dev_proxy).await
            } else {
                (None, None)
            };

            match dev_type {
                NM_DEVICE_TYPE_ETHERNET => {
                    let wired_proxy =
                        zbus::Proxy::new(&self.conn, NM_SERVICE, dev_path.as_str(), NM_WIRED_IFACE)
                            .await?;
                    let speed: u32 = wired_proxy.get_property("Speed").await.unwrap_or(1000);

                    let (conn_path, autoconnect) =
                        saved_conns.get(&iface_name).cloned().unwrap_or((None, true));

                    entries.push(NetworkEntry {
                        name: iface_name.clone(),
                        kind: NetworkKind::Wired {
                            interface: iface_name,
                            speed,
                        },
                        is_connected: is_dev_activated,
                        autoconnect,
                        ip_address,
                        gateway,
                        dbus_path: dev_path.as_str().to_string(),
                        connection_path: conn_path,
                    });
                }
                NM_DEVICE_TYPE_WIFI => {
                    let wifi_proxy = zbus::Proxy::new(
                        &self.conn,
                        NM_SERVICE,
                        dev_path.as_str(),
                        NM_WIRELESS_IFACE,
                    )
                    .await?;

                    let active_ap_path: Option<OwnedObjectPath> = if is_dev_activated {
                        wifi_proxy.get_property("ActiveAccessPoint").await.ok()
                    } else {
                        None
                    };

                    let active_ap_str = active_ap_path
                        .as_ref()
                        .map(|p| p.as_str().trim_end_matches('/').to_string())
                        .unwrap_or_default();

                    let active_ssid: Option<String> = if is_dev_activated {
                        if let Ok(ac_path) =
                            dev_proxy.get_property::<OwnedObjectPath>("ActiveConnection").await
                        {
                            if let Ok(ac_proxy) = zbus::Proxy::new(
                                &self.conn,
                                NM_SERVICE,
                                ac_path.as_str(),
                                "org.freedesktop.NetworkManager.ActiveConnection",
                            )
                            .await
                            {
                                ac_proxy.get_property::<String>("Id").await.ok()
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let ap_paths: Vec<OwnedObjectPath> = wifi_proxy
                        .call("GetAllAccessPoints", &())
                        .await
                        .unwrap_or_default();

                    let mut seen_ssids: HashMap<String, NetworkEntry> = HashMap::new();

                    for ap in ap_paths {
                        if let Ok(ap_proxy) =
                            zbus::Proxy::new(&self.conn, NM_SERVICE, ap.as_str(), NM_AP_IFACE).await
                        {
                            let raw_ssid: Vec<u8> =
                                ap_proxy.get_property("Ssid").await.unwrap_or_default();
                            let ssid = String::from_utf8_lossy(&raw_ssid).to_string();

                            if ssid.trim().is_empty() {
                                continue;
                            }

                            let strength: u8 =
                                ap_proxy.get_property("Strength").await.unwrap_or(0);
                            let hw_addr: String =
                                ap_proxy.get_property("HwAddress").await.unwrap_or_default();
                            let wpa_flags: u32 =
                                ap_proxy.get_property("WpaFlags").await.unwrap_or(0);
                            let rsn_flags: u32 =
                                ap_proxy.get_property("RsnFlags").await.unwrap_or(0);

                            let sec_type = parse_security(wpa_flags, rsn_flags);

                            let ap_clean = ap.as_str().trim_end_matches('/');
                            let is_connected = is_dev_activated
                                && ((!active_ap_str.is_empty() && ap_clean == active_ap_str)
                                    || active_ssid.as_deref() == Some(&ssid));

                            let (conn_path, autoconnect) =
                                saved_conns.get(&ssid).cloned().unwrap_or((None, false));

                            let entry = NetworkEntry {
                                name: ssid.clone(),
                                kind: NetworkKind::Wireless {
                                    ssid: ssid.clone(),
                                    bssid: hw_addr,
                                    signal: strength,
                                    security: sec_type,
                                },
                                is_connected,
                                autoconnect,
                                ip_address: if is_connected { ip_address.clone() } else { None },
                                gateway: if is_connected { gateway.clone() } else { None },
                                dbus_path: dev_path.as_str().to_string(),
                                connection_path: conn_path,
                            };

                            match seen_ssids.get(&ssid) {
                                Some(existing) => {
                                    if is_connected
                                        || (!existing.is_connected
                                            && strength
                                                > match &existing.kind {
                                                    NetworkKind::Wireless { signal, .. } => *signal,
                                                    _ => 0,
                                                })
                                    {
                                        seen_ssids.insert(ssid, entry);
                                    }
                                }
                                None => {
                                    seen_ssids.insert(ssid, entry);
                                }
                            }
                        }
                    }

                    for (_, net) in seen_ssids {
                        entries.push(net);
                    }
                }
                _ => {}
            }
        }

        entries.sort_by(|a, b| {
            b.is_connected
                .cmp(&a.is_connected)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(entries)
    }

    /// Extract IPv4 address and Gateway from IP4Config interface
    async fn get_ip_info(&self, dev_proxy: &zbus::Proxy<'_>) -> (Option<String>, Option<String>) {
        let ip4_path: Result<OwnedObjectPath, _> = dev_proxy.get_property("Ip4Config").await;
        if let Ok(path) = ip4_path {
            if path.as_str() == "/" {
                return (None, None);
            }
            if let Ok(ip_proxy) =
                zbus::Proxy::new(&self.conn, NM_SERVICE, path.as_str(), NM_IP4_IFACE).await
            {
                let gateway: Option<String> = ip_proxy.get_property("Gateway").await.ok();

                if let Ok(addr_data) =
                    ip_proxy.get_property::<Vec<HashMap<String, OwnedValue>>>("AddressData").await
                {
                    if let Some(first) = addr_data.first() {
                        let addr = first.get("address").and_then(|v| match &**v {
                            Value::Str(s) => Some(s.to_string()),
                            _ => None,
                        });
                        let pfx = first.get("prefix").and_then(|v| match &**v {
                            Value::U32(p) => Some(*p),
                            _ => None,
                        });
                        if let (Some(a), Some(p)) = (addr, pfx) {
                            return (Some(format!("{}/{}", a, p)), gateway);
                        }
                    }
                }
                return (None, gateway);
            }
        }
        (None, None)
    }

    /// Read saved connection settings to match SSIDs with UUIDs/paths and autoconnect state
    async fn get_saved_connections(
        &self,
    ) -> Result<HashMap<String, (Option<String>, bool)>, Box<dyn Error>> {
        let mut map = HashMap::new();
        let settings_proxy = zbus::Proxy::new(
            &self.conn,
            NM_SERVICE,
            "/org/freedesktop/NetworkManager/Settings",
            NM_SETTINGS_IFACE,
        )
        .await?;
        let conns: Vec<OwnedObjectPath> =
            settings_proxy.call("ListConnections", &()).await.unwrap_or_default();

        for cp in conns {
            let conn_proxy =
                zbus::Proxy::new(&self.conn, NM_SERVICE, cp.as_str(), NM_SETTINGS_CONN_IFACE)
                    .await?;
            let res: Result<HashMap<String, HashMap<String, OwnedValue>>, _> =
                conn_proxy.call("GetSettings", &()).await;

            if let Ok(settings) = res {
                let id = settings
                    .get("connection")
                    .and_then(|c| c.get("id"))
                    .and_then(|v| match &**v {
                        Value::Str(s) => Some(s.to_string()),
                        _ => None,
                    });

                let wifi_ssid = settings
                    .get("802-11-wireless")
                    .or_else(|| settings.get("wifi"))
                    .and_then(|w| w.get("ssid"))
                    .and_then(|v| match &**v {
                        Value::Array(arr) => {
                            let bytes: Vec<u8> = arr
                                .iter()
                                .filter_map(|b| match b {
                                    Value::U8(byte) => Some(*byte),
                                    _ => None,
                                })
                                .collect();
                            Some(String::from_utf8_lossy(&bytes).to_string())
                        }
                        _ => None,
                    });

                let autoconnect = settings
                    .get("connection")
                    .and_then(|c| c.get("autoconnect"))
                    .and_then(|v| match &**v {
                        Value::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(true);

                let path_str = cp.as_str().to_string();
                if let Some(name) = id {
                    map.insert(name, (Some(path_str.clone()), autoconnect));
                }
                if let Some(ssid) = wifi_ssid {
                    map.insert(ssid, (Some(path_str), autoconnect));
                }
            }
        }
        Ok(map)
    }

    /// Retrieve Wi-Fi password by prompting in an ephemeral floating terminal window
    pub async fn get_wifi_password(&self, conn_path: &str) -> Result<String, Box<dyn Error>> {
        // 1. First attempt unprivileged read
        if let Ok(conn_proxy) =
            zbus::Proxy::new(&self.conn, NM_SERVICE, conn_path, NM_SETTINGS_CONN_IFACE).await
        {
            for setting_name in &["802-11-wireless-security", "wifi-security"] {
                let res: Result<HashMap<String, HashMap<String, OwnedValue>>, _> =
                    conn_proxy.call("GetSecrets", &(setting_name,)).await;

                if let Ok(secrets) = res {
                    if let Some(sec) = secrets.get(*setting_name) {
                        for key in &["psk", "password", "wep-key0"] {
                            if let Some(val) = sec.get(*key).and_then(|v| match &**v {
                                Value::Str(s) => Some(s.to_string()),
                                _ => None,
                            }) {
                                if !val.is_empty() {
                                    return Ok(val);
                                }
                            }
                        }
                    }
                }
            }

            // Check plaintext settings fallback
            let res: Result<HashMap<String, HashMap<String, OwnedValue>>, _> =
                conn_proxy.call("GetSettings", &()).await;

            if let Ok(settings) = res {
                for sec_name in &["802-11-wireless-security", "wifi-security"] {
                    if let Some(sec) = settings.get(*sec_name) {
                        for key in &["psk", "password", "wep-key0"] {
                            if let Some(val) = sec.get(*key).and_then(|v| match &**v {
                                Value::Str(s) => Some(s.to_string()),
                                _ => None,
                            }) {
                                if !val.is_empty() {
                                    return Ok(val);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Spawn floating terminal prompt via RAM file (/dev/shm)
        let rand_id: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos() as u64;
        let tmp_file = format!("/dev/shm/.wtui_secret_{}", rand_id);

        let bash_script = format!(
            "echo -e '\\033[1;34m:: NetworkManager Auth Required\\033[0m'; \
             touch '{0}' && chmod 600 '{0}'; \
             if sudo nmcli -s -g 802-11-wireless-security.psk connection show '{1}' > '{0}'; then \
                 exit 0; \
             else \
                 echo -e '\\033[1;31mAuthentication failed or cancelled.\\033[0m'; \
                 sleep 1.2; \
                 exit 1; \
             fi",
            tmp_file, conn_path
        );

        let term = std::env::var("TERMINAL").unwrap_or_else(|_| {
            for t in &["kitty", "alacritty", "foot", "wezterm", "ghostty"] {
                if std::process::Command::new("which")
                    .arg(t)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    return t.to_string();
                }
            }
            "xterm".to_string()
        });

        let child = match term.as_str() {
            "kitty" => {
                tokio::process::Command::new("kitty")
                    .args(["--title", "wtui-auth", "--", "bash", "-c", &bash_script])
                    .status()
                    .await
            }
            "alacritty" => {
                tokio::process::Command::new("alacritty")
                    .args(["--title", "wtui-auth", "-e", "bash", "-c", &bash_script])
                    .status()
                    .await
            }
            "foot" => {
                tokio::process::Command::new("foot")
                    .args(["--title", "wtui-auth", "bash", "-c", &bash_script])
                    .status()
                    .await
            }
            _ => {
                tokio::process::Command::new(&term)
                    .args(["-e", "bash", "-c", &bash_script])
                    .status()
                    .await
            }
        };

        let mut password = String::new();

        if let Ok(status) = child {
            if status.success() {
                if let Ok(contents) = tokio::fs::read_to_string(&tmp_file).await {
                    password = contents.trim().to_string();
                }
            }
        }

        let _ = tokio::fs::remove_file(&tmp_file).await;

        Ok(password)
    }

    /// Toggle autoconnect on an existing connection profile
    pub async fn set_autoconnect(
        &self,
        conn_path: &str,
        enable: bool,
    ) -> Result<(), Box<dyn Error>> {
        let conn_proxy =
            zbus::Proxy::new(&self.conn, NM_SERVICE, conn_path, NM_SETTINGS_CONN_IFACE).await?;
        let mut settings: HashMap<String, HashMap<String, OwnedValue>> =
            conn_proxy.call("GetSettings", &()).await?;

        if let Some(c) = settings.get_mut("connection") {
            c.insert("autoconnect".to_string(), OwnedValue::from(enable));
            let _: () = conn_proxy.call("Update", &(settings,)).await?;
        }
        Ok(())
    }

    /// Trigger a Wi-Fi scan
    pub async fn request_wireless_scan(&self) -> Result<(), Box<dyn Error>> {
        let nm_proxy = zbus::Proxy::new(&self.conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;
        let devices: Vec<OwnedObjectPath> = nm_proxy.call("GetDevices", &()).await?;

        for d in devices {
            let dev =
                zbus::Proxy::new(&self.conn, NM_SERVICE, d.as_str(), NM_DEV_IFACE).await?;
            let dev_type: u32 = dev.get_property("DeviceType").await.unwrap_or(0);
            if dev_type == NM_DEVICE_TYPE_WIFI {
                let wifi = zbus::Proxy::new(
                    &self.conn,
                    NM_SERVICE,
                    d.as_str(),
                    NM_WIRELESS_IFACE,
                )
                .await?;
                let empty_options: HashMap<String, Value> = HashMap::new();
                let _: () = wifi.call("RequestScan", &(empty_options,)).await?;
            }
        }
        Ok(())
    }

    /// Connect to an existing connection profile
    pub async fn connect_network(&self, entry: &NetworkEntry) -> Result<(), Box<dyn Error>> {
        let nm_proxy = zbus::Proxy::new(&self.conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;
        if let Some(cp) = &entry.connection_path {
            let conn_obj = ObjectPath::try_from(cp.as_str())?;
            let dev_obj = ObjectPath::try_from(entry.dbus_path.as_str())?;
            let specific_obj = ObjectPath::try_from("/")?;

            let _: OwnedObjectPath = nm_proxy
                .call("ActivateConnection", &(&conn_obj, &dev_obj, &specific_obj))
                .await?;
        }
        Ok(())
    }

    /// Disconnect an active device
    pub async fn disconnect_network(&self, dev_path: &str) -> Result<(), Box<dyn Error>> {
        let dev = zbus::Proxy::new(&self.conn, NM_SERVICE, dev_path, NM_DEV_IFACE).await?;
        let _: () = dev.call("Disconnect", &()).await?;
        Ok(())
    }
}

fn parse_security(wpa_flags: u32, rsn_flags: u32) -> String {
    if rsn_flags & NM_802_11_AP_SEC_KEY_MGMT_SAE != 0 {
        "WPA3-SAE".to_string()
    } else if (rsn_flags != NM_802_11_AP_SEC_NONE) || (wpa_flags != NM_802_11_AP_SEC_NONE) {
        if (rsn_flags & NM_802_11_AP_SEC_KEY_MGMT_802_1X != 0)
            || (wpa_flags & NM_802_11_AP_SEC_KEY_MGMT_802_1X != 0)
        {
            "WPA-Enterprise".to_string()
        } else {
            "WPA2-PSK".to_string()
        }
    } else {
        "Open".to_string()
    }
}
