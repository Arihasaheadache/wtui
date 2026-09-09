use crate::app::{App, NetworkKind};
use crate::qr::generate_wifi_qr_lines;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

pub struct Theme;

impl Theme {
    // Preserve terminal alpha transparency for Hyprland compositor blur
    pub const BG: Color = Color::Reset;

    // Catppuccin Mocha palette
    pub const BORDER: Color = Color::Rgb(108, 112, 134);       // Slate Muted
    pub const PRIMARY: Color = Color::Rgb(137, 180, 250);      // Soft Blue
    pub const ACCENT: Color = Color::Rgb(203, 166, 247);       // Mauve
    pub const SUCCESS: Color = Color::Rgb(166, 227, 161);      // Green
    pub const WARNING: Color = Color::Rgb(249, 226, 175);      // Yellow
    pub const TEXT: Color = Color::Rgb(205, 214, 244);         // Foreground White
    pub const MUTED: Color = Color::Rgb(147, 153, 178);        // Dim Text
    pub const HIGHLIGHT: Color = Color::Rgb(49, 50, 68);       // Surface / Row Selection

    pub fn container_block(title: &'static str) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Self::BORDER))
            .style(Style::default().bg(Self::BG))
            .title(format!(" {title} "))
            .title_style(Style::default().fg(Self::PRIMARY).add_modifier(Modifier::BOLD))
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // 1. Primary Horizontal Split: Left 50% (Panels), Right 50% (QR Pane)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(size);

    // 2. Sub-divide Left side vertically: 50% Network List, 50% Detailed Controls
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(main_chunks[0]);

    render_network_list(f, app, left_chunks[0]);
    render_network_details(f, app, left_chunks[1]);
    render_qr_pane(f, app, main_chunks[1]);
}

fn signal_meter(pct: u8) -> &'static str {
    match pct {
        0..=24 => "▂___",
        25..=49 => "▂▄__",
        50..=74 => "▂▄▆_",
        _ => "▂▄▆█",
    }
}

fn render_network_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .networks
        .iter()
        .enumerate()
        .map(|(idx, net)| {
            let is_selected = idx == app.selected_index;
            let status_prefix = if net.is_connected { "(c) " } else { "    " };

            let prefix_color = if net.is_connected {
                Theme::SUCCESS
            } else {
                Theme::MUTED
            };

            let (kind_str, signal_str) = match &net.kind {
                NetworkKind::Wired { speed, .. } => {
                    (format!("Eth [{}M]", speed), String::new())
                }
                NetworkKind::Wireless { signal, .. } => (
                    "Wi-Fi".to_string(),
                    format!("{}% {}", signal, signal_meter(*signal)),
                ),
            };

            let line = Line::from(vec![
                Span::styled(
                    status_prefix,
                    Style::default().fg(prefix_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:<20}", net.name),
                    Style::default().fg(if is_selected {
                        Theme::ACCENT
                    } else {
                        Theme::TEXT
                    }),
                ),
                Span::styled(
                    format!("{:>8} ", kind_str),
                    Style::default().fg(Theme::MUTED),
                ),
                Span::styled(
                    signal_str,
                    Style::default().fg(Theme::PRIMARY),
                ),
            ]);

            let style = if is_selected {
                Style::default()
                    .bg(Theme::HIGHLIGHT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(Theme::BG)
            };

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Theme::container_block("Networks"))
        .style(Style::default().bg(Theme::BG));

    f.render_widget(list, area);
}

fn render_network_details(f: &mut Frame, app: &App, area: Rect) {
    let block = Theme::container_block("Details & Controls");

    if let Some(net) = app.selected_network() {
        let (sec_type, bssid_val) = match &net.kind {
            NetworkKind::Wireless { security, bssid, .. } => (security.as_str(), bssid.as_str()),
            NetworkKind::Wired { .. } => ("802.3 Wired", "N/A"),
        };

        let auto_color = if net.autoconnect {
            Theme::SUCCESS
        } else {
            Theme::MUTED
        };
        let auto_str = if net.autoconnect { "ENABLED" } else { "DISABLED" };

        let details = vec![
            Line::from(vec![
                Span::styled("  Name:     ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(&net.name, Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("  Security: ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(sec_type, Style::default().fg(Theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  BSSID:    ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(bssid_val, Style::default().fg(Theme::MUTED)),
            ]),
            Line::from(vec![
                Span::styled("  IPv4:     ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(
                    net.ip_address.as_deref().unwrap_or("No IP assigned"),
                    Style::default().fg(Theme::TEXT),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Gateway:  ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(
                    net.gateway.as_deref().unwrap_or("None"),
                    Style::default().fg(Theme::MUTED),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Auto-Connect: ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(auto_str, Style::default().fg(auto_color).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [c] ", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(
                    if net.is_connected { "Disconnect" } else { "Connect" },
                    Style::default().fg(Theme::TEXT),
                ),
                Span::styled("   [a] ", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled("Toggle Auto", Style::default().fg(Theme::TEXT)),
                Span::styled("   [s] ", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled("QR Code", Style::default().fg(Theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  [r] ", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled("Rescan", Style::default().fg(Theme::TEXT)),
                Span::styled("       [q] ", Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled("Quit", Style::default().fg(Theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Status: ", Style::default().fg(Theme::MUTED)),
                Span::styled(&app.status_message, Style::default().fg(Theme::WARNING)),
            ]),
        ];

        let p = Paragraph::new(details)
            .block(block)
            .style(Style::default().bg(Theme::BG))
            .wrap(Wrap { trim: true });

        f.render_widget(p, area);
    } else {
        let empty = Paragraph::new("No networks detected.\nPress [r] to rescan.")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Theme::MUTED).bg(Theme::BG));
        f.render_widget(empty, area);
    }
}

fn render_qr_pane(f: &mut Frame, app: &App, area: Rect) {
    let block = Theme::container_block("Wi-Fi Share");

    if !app.show_qr {
        let help_text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Wi-Fi Sharing Inactive",
                Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Select a wireless network and press [s]",
                Style::default().fg(Theme::TEXT),
            )),
            Line::from(Span::styled(
                "to generate a mobile connection QR code.",
                Style::default().fg(Theme::MUTED),
            )),
        ];

        let p = Paragraph::new(help_text)
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().bg(Theme::BG));

        f.render_widget(p, area);
        return;
    }

    if let Some(net) = app.selected_network() {
        if let NetworkKind::Wireless { security, .. } = &net.kind {
            let password = app.qr_password.as_deref().unwrap_or("");
            let mut qr_lines = generate_wifi_qr_lines(&net.name, password, security);

            // Detailed preview lines showing actual extracted credentials
            qr_lines.push(Line::from(""));
            qr_lines.push(Line::from(vec![
                Span::styled("  SSID: ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(&net.name, Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
            ]));
            qr_lines.push(Line::from(vec![
                Span::styled("  Pass: ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(
                    if password.is_empty() {
                        "<None / Permission Denied>"
                    } else {
                        password
                    },
                    Style::default().fg(if password.is_empty() {
                        Theme::WARNING
                    } else {
                        Theme::SUCCESS
                    }),
                ),
            ]));

            let p = Paragraph::new(qr_lines)
                .block(block)
                .alignment(Alignment::Center)
                .style(Style::default().bg(Theme::BG));

            f.render_widget(p, area);
            return;
        }
    }

    let unsupported = Paragraph::new("QR code only available for Wi-Fi networks.")
        .block(block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Theme::MUTED).bg(Theme::BG));

    f.render_widget(unsupported, area);
}
