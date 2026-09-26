use qrcode::{EcLevel, QrCode};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

const QUIET_ZONE: usize = 2;

const COLOR_WHITE: Color = Color::Rgb(255, 255, 255);
const COLOR_BLACK: Color = Color::Rgb(0, 0, 0);

fn escape_wifi_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());

    for c in s.chars() {
        if matches!(c, '\\' | ';' | ':' | ',' | '"') {
            out.push('\\');
        }

        out.push(c);
    }

    out
}

pub fn generate_wifi_qr_lines(ssid: &str, password: &str, security: &str) -> Vec<Line<'static>> {
    let upper_security = security.to_uppercase();

    let auth_type = if upper_security.contains("WPA") {
        "WPA"
    } else if upper_security.contains("WEP") {
        "WEP"
    } else {
        "nopass"
    };

    let escaped_ssid = escape_wifi_string(ssid);
    let escaped_pwd = escape_wifi_string(password);

    let payload = if auth_type == "nopass" {
        format!("WIFI:T:nopass;S:{escaped_ssid};;")
    } else {
        format!("WIFI:T:{auth_type};S:{escaped_ssid};P:{escaped_pwd};;")
    };

    let code = match QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M) {
        Ok(c) => c,
        Err(_) => match QrCode::new(payload.as_bytes()) {
            Ok(c) => c,
            Err(_) => return vec![Line::from("Failed to generate QR code.")],
        },
    };

    let width = code.width();

    let modules: Vec<bool> = code
        .into_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();

    let padded_width = width + (QUIET_ZONE * 2);

    let mut lines = Vec::new();

    let white_cell = Span::styled("▀", Style::default().fg(COLOR_WHITE).bg(COLOR_WHITE));

    lines.push(Line::from(vec![white_cell.clone(); padded_width]));

    for y in (0..width).step_by(2) {
        let mut spans = Vec::with_capacity(padded_width);

        for _ in 0..QUIET_ZONE {
            spans.push(white_cell.clone());
        }

        for x in 0..width {
            let top_is_dark = modules[y * width + x];

            let bottom_is_dark = if y + 1 < width {
                modules[(y + 1) * width + x]
            } else {
                false
            };

            let (fg, bg) = match (top_is_dark, bottom_is_dark) {
                (true, true) => (COLOR_BLACK, COLOR_BLACK),
                (true, false) => (COLOR_BLACK, COLOR_WHITE),
                (false, true) => (COLOR_WHITE, COLOR_BLACK),
                (false, false) => (COLOR_WHITE, COLOR_WHITE),
            };

            spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
        }

        for _ in 0..QUIET_ZONE {
            spans.push(white_cell.clone());
        }

        lines.push(Line::from(spans));
    }

    lines.push(Line::from(vec![white_cell.clone(); padded_width]));

    lines
}
