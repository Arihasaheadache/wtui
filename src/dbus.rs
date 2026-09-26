use crate::app::{NetworkEntry, NetworkKind};
use std::collections::HashMap;
use std::error::Error;
use std::time::{Duration, Instant};
use zbus::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

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
const NM_ACTIVE_CONN_IFACE: &str = "org.freedesktop.NetworkManager.ActiveConnection";

const NM_DEVICE_TYPE_ETHERNET: u32 = 1;
const NM_DEVICE_TYPE_WIFI: u32 = 2;

const NM_802_11_AP_SEC_NONE: u32 = 0x0;
const NM_802_11_AP_SEC_KEY_MGMT_PSK: u32 = 0x100;
const NM_802_11_AP_SEC_KEY_MGMT_802_1X: u32 = 0x200;
const NM_802_11_AP_SEC_KEY_MGMT_SAE: u32 = 0x400;

#[derive(Clone)]
pub struct NmClient {
    conn: Connection,
}

fn owned<'a>(value: Value<'a>) -> Result<OwnedValue, Box<dyn Error>> {
    OwnedValue::try_from(value).map_err(|e| format!("D-Bus value conversion error: {e}").into())
}

fn random_uuid() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let pid = std::process::id() as u128;

    let a = (now & 0xffff_ffff) as u32;
    let b = ((now >> 32) & 0xffff) as u16;
    let c = ((now >> 48) & 0x0fff) as u16;
    let d = ((pid ^ (now >> 64)) & 0xffff) as u16;
    let e = ((now >> 80) & 0xffff_ffff_ffff) as u64;

    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        a,
        b,
        c | 0x4000,
        d | 0x8000,
        e
    )
}

fn value_to_string(v: &OwnedValue) -> Option<String> {
    match &**v {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

fn value_to_ssid_string(v: &OwnedValue) -> Option<String> {
    match &**v {
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
    }
}

fn infer_key_mgmt(security: &str) -> String {
    if security.contains("SAE") {
        "sae".to_string()
    } else if security.contains("WEP") {
        "none".to_string()
    } else {
        "wpa-psk".to_string()
    }
}

fn validate_wifi_password(security: &str, password: &str) -> Result<(), Box<dyn Error>> {
    if security.contains("Open") {
        return Ok(());
    }

    if security.contains("Enterprise") {
        return Err("WPA-Enterprise is not supported by this TUI yet".into());
    }

    if security.contains("WEP") {
        return Ok(());
    }

    if security.contains("SAE") {
        return Ok(());
    }

    if password.len() == 64 {
        return Ok(());
    }

    if password.len() >= 8 && password.len() <= 63 {
        return Ok(());
    }

    Err("WPA/WPA2 password must be 8-63 characters, or exactly 64 hex characters".into())
}

impl NmClient {
    pub async fn new() -> Result<Self, Box<dyn Error>> {
        let conn = Connection::system().await?;
        Ok(Self { conn })
    }

    pub async fn fetch_all_networks(&self) -> Result<Vec<NetworkEntry>, Box<dyn Error>> {
        let mut entries = Vec::new();

        let saved_conns = self.get_saved_connections().await.unwrap_or_default();

        let nm_proxy = zbus::Proxy::new(&self.conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;
        let devices: Vec<OwnedObjectPath> = nm_proxy.call("GetDevices", &()).await?;

        for dev_path in devices {
            let dev_proxy =
                zbus::Proxy::new(&self.conn, NM_SERVICE, dev_path.as_str(), NM_DEV_IFACE).await?;

            let dev_type: u32 = dev_proxy.get_property("DeviceType").await.unwrap_or(0);
            let dev_state: u32 = dev_proxy.get_property("State").await.unwrap_or(0);
            let iface_name: String = dev_proxy
                .get_property("Interface")
                .await
                .unwrap_or_default();

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

                    let (conn_path, autoconnect) = saved_conns
                        .get(&iface_name)
                        .cloned()
                        .unwrap_or((None, true));

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
                        ap_path: None,
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
                        match wifi_proxy
                            .get_property::<OwnedObjectPath>("ActiveAccessPoint")
                            .await
                        {
                            Ok(path) if path.as_str() != "/" => Some(path),
                            _ => None,
                        }
                    } else {
                        None
                    };

                    let active_ap_str = active_ap_path
                        .as_ref()
                        .map(|p| p.as_str().trim_end_matches('/').to_string())
                        .unwrap_or_default();

                    let active_ssid: Option<String> = if is_dev_activated {
                        match dev_proxy
                            .get_property::<OwnedObjectPath>("ActiveConnection")
                            .await
                        {
                            Ok(ac_path) if ac_path.as_str() != "/" => {
                                match zbus::Proxy::new(
                                    &self.conn,
                                    NM_SERVICE,
                                    ac_path.as_str(),
                                    NM_ACTIVE_CONN_IFACE,
                                )
                                .await
                                {
                                    Ok(ac_proxy) => ac_proxy.get_property("Id").await.ok(),
                                    Err(_) => None,
                                }
                            }
                            _ => None,
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

                            let strength: u8 = ap_proxy.get_property("Strength").await.unwrap_or(0);

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
                                ip_address: if is_connected {
                                    ip_address.clone()
                                } else {
                                    None
                                },
                                gateway: if is_connected { gateway.clone() } else { None },
                                dbus_path: dev_path.as_str().to_string(),
                                connection_path: conn_path,
                                ap_path: Some(ap.as_str().to_string()),
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

                if let Ok(addr_data) = ip_proxy
                    .get_property::<Vec<HashMap<String, OwnedValue>>>("AddressData")
                    .await
                {
                    if let Some(first) = addr_data.first() {
                        let addr = first.get("address").and_then(|v| match &**v {
                            Value::Str(s) => Some(s.as_str().to_string()),
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

        let conns: Vec<OwnedObjectPath> = settings_proxy
            .call("ListConnections", &())
            .await
            .unwrap_or_default();

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
                    .and_then(value_to_string);

                let uuid = settings
                    .get("connection")
                    .and_then(|c| c.get("uuid"))
                    .and_then(value_to_string);

                let interface_name = settings
                    .get("connection")
                    .and_then(|c| c.get("interface-name"))
                    .and_then(value_to_string);

                let wifi_ssid = settings
                    .get("802-11-wireless")
                    .or_else(|| settings.get("wifi"))
                    .and_then(|w| w.get("ssid"))
                    .and_then(value_to_ssid_string);

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

                if let Some(uuid) = uuid {
                    map.insert(uuid, (Some(path_str.clone()), autoconnect));
                }

                if let Some(interface_name) = interface_name {
                    map.insert(interface_name, (Some(path_str.clone()), autoconnect));
                }

                if let Some(ssid) = wifi_ssid {
                    map.insert(ssid, (Some(path_str), autoconnect));
                }
            }
        }

        Ok(map)
    }

    pub async fn get_wifi_password(&self, conn_path: &str) -> Result<String, Box<dyn Error>> {
        let conn_proxy =
            zbus::Proxy::new(&self.conn, NM_SERVICE, conn_path, NM_SETTINGS_CONN_IFACE).await?;

        for setting_name in ["802-11-wireless-security", "wifi-security"] {
            let res: Result<HashMap<String, HashMap<String, OwnedValue>>, _> =
                conn_proxy.call("GetSecrets", &(setting_name,)).await;

            if let Ok(secrets) = res {
                if let Some(sec) = secrets.get(setting_name) {
                    for key in ["psk", "password", "wep-key0"] {
                        if let Some(val) = sec.get(key).and_then(value_to_string) {
                            if !val.is_empty() {
                                return Ok(val);
                            }
                        }
                    }
                }
            }
        }

        let mut conn_uuid: Option<String> = None;
        let mut conn_id: Option<String> = None;

        let settings_result: Result<HashMap<String, HashMap<String, OwnedValue>>, _> =
            conn_proxy.call("GetSettings", &()).await;

        if let Ok(settings) = settings_result {
            for sec_name in ["802-11-wireless-security", "wifi-security"] {
                if let Some(sec) = settings.get(sec_name) {
                    for key in ["psk", "password", "wep-key0"] {
                        if let Some(val) = sec.get(key).and_then(value_to_string) {
                            if !val.is_empty() {
                                return Ok(val);
                            }
                        }
                    }
                }
            }

            conn_id = settings
                .get("connection")
                .and_then(|c| c.get("id"))
                .and_then(value_to_string);

            conn_uuid = settings
                .get("connection")
                .and_then(|c| c.get("uuid"))
                .and_then(value_to_string);
        }

        let target = conn_uuid
            .or(conn_id)
            .ok_or("Could not determine connection UUID or ID")?;

        let target_escaped = target.replace('\'', "'\\''");

        let rand_id: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos() as u64;

        let tmp_file = format!("/dev/shm/.wtui_secret_{}", rand_id);

        let bash_script = format!(
            "echo -e '\\033[1;34m:: NetworkManager Auth Required\\033[0m'; \
             touch '{1}' && chmod 600 '{1}'; \
             if sudo nmcli -s -g 802-11-wireless-security.psk connection show '{0}' > '{1}'; then \
                 exit 0; \
             else \
                 echo -e '\\033[1;31mAuthentication failed or cancelled.\\033[0m'; \
                 sleep 1.2; \
                 exit 1; \
             fi",
            target_escaped, tmp_file
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

        if password.is_empty() {
            return Err("Password prompt failed or returned empty".into());
        }

        Ok(password)
    }

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
            c.insert("autoconnect".to_string(), owned(Value::from(enable))?);

            let _: () = conn_proxy.call("Update", &(settings,)).await?;
        }

        Ok(())
    }

    pub async fn request_wireless_scan(&self) -> Result<(), Box<dyn Error>> {
        let nm_proxy = zbus::Proxy::new(&self.conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;

        let devices: Vec<OwnedObjectPath> = nm_proxy.call("GetDevices", &()).await?;

        for d in devices {
            let dev = zbus::Proxy::new(&self.conn, NM_SERVICE, d.as_str(), NM_DEV_IFACE).await?;

            let dev_type: u32 = dev.get_property("DeviceType").await.unwrap_or(0);

            if dev_type == NM_DEVICE_TYPE_WIFI {
                let wifi =
                    zbus::Proxy::new(&self.conn, NM_SERVICE, d.as_str(), NM_WIRELESS_IFACE).await?;

                let empty_options: HashMap<String, OwnedValue> = HashMap::new();

                let _: () = wifi.call("RequestScan", &(empty_options,)).await?;
            }
        }

        Ok(())
    }

    pub async fn disconnect_network(&self, dev_path: &str) -> Result<(), Box<dyn Error>> {
        let dev = zbus::Proxy::new(&self.conn, NM_SERVICE, dev_path, NM_DEV_IFACE).await?;

        let _: () = dev.call("Disconnect", &()).await?;

        Ok(())
    }

    pub async fn connect_or_add_network(
        &self,
        entry: &NetworkEntry,
        password: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        match &entry.kind {
            NetworkKind::Wireless { ssid, security, .. } => {
                if security.contains("Enterprise") {
                    return Err("WPA-Enterprise is not supported by this TUI yet".into());
                }

                if let Some(pwd) = password {
                    validate_wifi_password(security, pwd)?;
                }

                let existing_path = if let Some(path) = &entry.connection_path {
                    Some(path.clone())
                } else {
                    self.find_saved_wifi_connection(ssid).await?
                };

                if let Some(conn_path) = existing_path {
                    if let Some(pwd) = password {
                        self.update_wifi_password(&conn_path, pwd, security).await?;
                    }

                    return self.activate_connection_path(&conn_path, entry).await;
                }

                self.create_and_activate_wifi(entry, ssid, security, password)
                    .await
            }

            _ => {
                if let Some(conn_path) = &entry.connection_path {
                    self.activate_connection_path(conn_path, entry).await
                } else {
                    Err("No saved connection profile available".into())
                }
            }
        }
    }

    pub async fn wait_for_connected(
        &self,
        name: &str,
        timeout: Duration,
    ) -> Result<(), Box<dyn Error>> {
        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Ok(networks) = self.fetch_all_networks().await {
                if networks.iter().any(|n| n.name == name && n.is_connected) {
                    return Ok(());
                }
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err("timed out waiting for NetworkManager to activate the connection".into())
    }

    pub async fn wait_for_disconnected(
        &self,
        name: &str,
        timeout: Duration,
    ) -> Result<(), Box<dyn Error>> {
        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Ok(networks) = self.fetch_all_networks().await {
                let still_connected = networks.iter().any(|n| n.name == name && n.is_connected);

                if !still_connected {
                    return Ok(());
                }
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err("timed out waiting for NetworkManager to disconnect".into())
    }

    async fn activate_connection_path(
        &self,
        conn_path: &str,
        entry: &NetworkEntry,
    ) -> Result<(), Box<dyn Error>> {
        let nm_proxy = zbus::Proxy::new(&self.conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;

        let conn_obj = ObjectPath::try_from(conn_path)?;
        let dev_obj = ObjectPath::try_from(entry.dbus_path.as_str())?;

        let specific_path = match &entry.kind {
            NetworkKind::Wireless { .. } => entry
                .ap_path
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("/"),
            _ => "/",
        };

        let specific_obj = ObjectPath::try_from(specific_path)?;

        let _: OwnedObjectPath = nm_proxy
            .call("ActivateConnection", &(&conn_obj, &dev_obj, &specific_obj))
            .await?;

        Ok(())
    }

    async fn find_saved_wifi_connection(
        &self,
        ssid: &str,
    ) -> Result<Option<String>, Box<dyn Error>> {
        let settings_proxy = zbus::Proxy::new(
            &self.conn,
            NM_SERVICE,
            "/org/freedesktop/NetworkManager/Settings",
            NM_SETTINGS_IFACE,
        )
        .await?;

        let conns: Vec<OwnedObjectPath> = settings_proxy
            .call("ListConnections", &())
            .await
            .unwrap_or_default();

        for cp in conns {
            let conn_proxy =
                match zbus::Proxy::new(&self.conn, NM_SERVICE, cp.as_str(), NM_SETTINGS_CONN_IFACE)
                    .await
                {
                    Ok(proxy) => proxy,
                    Err(_) => continue,
                };

            let settings: Result<HashMap<String, HashMap<String, OwnedValue>>, _> =
                conn_proxy.call("GetSettings", &()).await;

            let settings = match settings {
                Ok(s) => s,
                Err(_) => continue,
            };

            let conn_type = settings
                .get("connection")
                .and_then(|c| c.get("type"))
                .and_then(value_to_string);

            if conn_type.as_deref() != Some("802-11-wireless") {
                continue;
            }

            let wifi_ssid = settings
                .get("802-11-wireless")
                .or_else(|| settings.get("wifi"))
                .and_then(|w| w.get("ssid"))
                .and_then(value_to_ssid_string);

            if wifi_ssid.as_deref() == Some(ssid) {
                return Ok(Some(cp.as_str().to_string()));
            }
        }

        Ok(None)
    }

    async fn update_wifi_password(
        &self,
        conn_path: &str,
        password: &str,
        security: &str,
    ) -> Result<(), Box<dyn Error>> {
        let conn_proxy =
            zbus::Proxy::new(&self.conn, NM_SERVICE, conn_path, NM_SETTINGS_CONN_IFACE).await?;

        let mut settings: HashMap<String, HashMap<String, OwnedValue>> =
            conn_proxy.call("GetSettings", &()).await?;

        let sec = settings
            .entry("802-11-wireless-security".to_string())
            .or_default();

        let current_key_mgmt = sec
            .get("key-mgmt")
            .and_then(value_to_string)
            .unwrap_or_else(|| infer_key_mgmt(security));

        sec.insert(
            "key-mgmt".to_string(),
            owned(Value::from(current_key_mgmt.clone()))?,
        );

        if current_key_mgmt == "none" {
            sec.insert(
                "wep-key0".to_string(),
                owned(Value::from(password.to_string()))?,
            );

            sec.insert("wep-key-type".to_string(), owned(Value::from(1u32))?);
        } else {
            sec.insert("psk".to_string(), owned(Value::from(password.to_string()))?);
        }

        let _: () = conn_proxy.call("Update", &(settings,)).await?;

        Ok(())
    }

    async fn create_and_activate_wifi(
        &self,
        entry: &NetworkEntry,
        ssid: &str,
        security: &str,
        password: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        if security != "Open" && password.as_ref().map(|p| p.is_empty()).unwrap_or(true) {
            return Err("A password is required for this network".into());
        }

        if let Some(pwd) = password {
            validate_wifi_password(security, pwd)?;
        }

        let key_mgmt = if security == "Open" {
            String::new()
        } else {
            infer_key_mgmt(security)
        };

        let mut settings: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();

        let mut connection = HashMap::new();

        // Important: no "(wtui)" suffix.
        connection.insert("id".to_string(), owned(Value::from(ssid.to_string()))?);

        connection.insert("uuid".to_string(), owned(Value::from(random_uuid()))?);

        connection.insert("type".to_string(), owned(Value::from("802-11-wireless"))?);

        connection.insert("autoconnect".to_string(), owned(Value::from(true))?);

        let mut wifi = HashMap::new();

        wifi.insert(
            "ssid".to_string(),
            owned(Value::from(ssid.as_bytes().to_vec()))?,
        );

        wifi.insert("mode".to_string(), owned(Value::from("infrastructure"))?);

        settings.insert("connection".to_string(), connection);
        settings.insert("802-11-wireless".to_string(), wifi);

        if security != "Open" {
            let mut security_settings = HashMap::new();

            security_settings.insert(
                "key-mgmt".to_string(),
                owned(Value::from(key_mgmt.clone()))?,
            );

            if key_mgmt == "none" {
                security_settings.insert(
                    "wep-key0".to_string(),
                    owned(Value::from(password.unwrap_or_default().to_string()))?,
                );

                security_settings.insert("wep-key-type".to_string(), owned(Value::from(1u32))?);
            } else {
                security_settings.insert(
                    "psk".to_string(),
                    owned(Value::from(password.unwrap_or_default().to_string()))?,
                );
            }

            settings.insert("802-11-wireless-security".to_string(), security_settings);
        }

        let nm_proxy = zbus::Proxy::new(&self.conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;

        let device_obj = ObjectPath::try_from(entry.dbus_path.as_str())?;

        let ap_path = entry
            .ap_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("/");

        let ap_obj = ObjectPath::try_from(ap_path)?;

        let _: (OwnedObjectPath, OwnedObjectPath) = nm_proxy
            .call("AddAndActivateConnection", &(settings, device_obj, ap_obj))
            .await?;

        Ok(())
    }
}

fn parse_security(wpa_flags: u32, rsn_flags: u32) -> String {
    if (rsn_flags & NM_802_11_AP_SEC_KEY_MGMT_802_1X != 0)
        || (wpa_flags & NM_802_11_AP_SEC_KEY_MGMT_802_1X != 0)
    {
        return "WPA-Enterprise".to_string();
    }

    if (rsn_flags & NM_802_11_AP_SEC_KEY_MGMT_PSK != 0)
        || (wpa_flags & NM_802_11_AP_SEC_KEY_MGMT_PSK != 0)
    {
        return "WPA2-PSK".to_string();
    }

    if rsn_flags & NM_802_11_AP_SEC_KEY_MGMT_SAE != 0 {
        return "WPA3-SAE".to_string();
    }

    if (rsn_flags != NM_802_11_AP_SEC_NONE) || (wpa_flags != NM_802_11_AP_SEC_NONE) {
        return "WPA2-PSK".to_string();
    }

    "Open".to_string()
}
