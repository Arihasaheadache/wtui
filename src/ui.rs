use crate::app::{App, NetworkKind, ViewMode};
use crate::qr::generate_wifi_qr_lines;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

pub struct Theme;
impl Theme {
    pub const BG: Color = Color::Reset;
    pub const BORDER: Color = Color::Rgb(108, 112, 134);
    pub const PRIMARY: Color = Color::Rgb(137, 180, 250);
    pub const ACCENT: Color = Color::Rgb(203, 166, 247);
    pub const SUCCESS: Color = Color::Rgb(166, 227, 161);
    pub const WARNING: Color = Color::Rgb(249, 226, 175);
    pub const TEXT: Color = Color::Rgb(205, 214, 244);
    pub const MUTED: Color = Color::Rgb(147, 153, 178);
    pub const HIGHLIGHT: Color = Color::Rgb(49, 50, 68);

    pub fn container_block(title: &'static str) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Self::BORDER))
            .style(Style::default().bg(Self::BG))
            .title(format!(" {title} "))
            .title_style(
                Style::default()
                    .fg(Self::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let size = f.area();
    if app.view_mode == ViewMode::SpeedTest {
        render_speed_test(f, app, size);
        return;
    }
    
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(size);
        
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[0]);
        
    render_network_list(f, app, left_chunks[0]);
    render_network_details(f, app, left_chunks[1]);
    render_qr_pane(f, app, main_chunks[1]);
    
    if app.password_modal.open {
        render_password_modal(f, app, size);
    }
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
                NetworkKind::Wired { speed, .. } => (format!("Eth [{}M]", speed), String::new()),
                NetworkKind::Wireless { signal, .. } => (
                    "Wi-Fi".to_string(),
                    format!("{}% {}", signal, signal_meter(*signal)),
                ),
            };
            let line = Line::from(vec![
                Span::styled(
                    status_prefix,
                    Style::default()
                        .fg(prefix_color)
                        .add_modifier(Modifier::BOLD),
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
                Span::styled(signal_str, Style::default().fg(Theme::PRIMARY)),
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
            NetworkKind::Wireless {
                security, bssid, ..
            } => (security.as_str(), bssid.as_str()),
            NetworkKind::Wired { .. } => ("802.3 Wired", "N/A"),
        };
        let auto_color = if net.autoconnect {
            Theme::SUCCESS
        } else {
            Theme::MUTED
        };
        let auto_str = if net.autoconnect {
            "ENABLED"
        } else {
            "DISABLED"
        };
        let details = vec![
            Line::from(vec![
                Span::styled("  Name:     ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(
                    &net.name,
                    Style::default()
                        .fg(Theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
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
                Span::styled(
                    auto_str,
                    Style::default().fg(auto_color).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "  [c] ",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if net.is_connected {
                        "Disconnect"
                    } else {
                        "Connect"
                    },
                    Style::default().fg(Theme::TEXT),
                ),
                Span::styled(
                    "   [a] ",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Toggle Auto", Style::default().fg(Theme::TEXT)),
                Span::styled(
                    "   [s] ",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("QR Code", Style::default().fg(Theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled(
                    "  [n] ",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Speed Test", Style::default().fg(Theme::TEXT)),
                Span::styled(
                    "    [r] ",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Rescan", Style::default().fg(Theme::TEXT)),
                Span::styled(
                    "        [q] ",
                    Style::default()
                        .fg(Theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
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
                Style::default()
                    .fg(Theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
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
            qr_lines.push(Line::from(""));
            qr_lines.push(Line::from(vec![
                Span::styled("  SSID: ", Style::default().fg(Theme::PRIMARY)),
                Span::styled(
                    &net.name,
                    Style::default()
                        .fg(Theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
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

fn render_password_modal(f: &mut Frame, app: &App, area: Rect) {
    let popup_area = centered_rect(50, 35, area);
    f.render_widget(Clear, popup_area);
    let ssid = app
        .password_modal
        .entry
        .as_ref()
        .map(|e| e.name.clone())
        .unwrap_or_default();
    let masked_password = "*".repeat(app.password_modal.input.chars().count());
    let mut lines = vec![
        Line::from(vec![
            Span::styled("SSID: ", Style::default().fg(Theme::PRIMARY)),
            Span::styled(
                ssid,
                Style::default()
                    .fg(Theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Password:",
            Style::default().fg(Theme::PRIMARY),
        )),
        Line::from(Span::styled(
            masked_password,
            Style::default()
                .fg(Theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    if let Some(error) = &app.password_modal.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: connect   Esc: cancel",
        Style::default().fg(Theme::MUTED),
    )));
    let paragraph = Paragraph::new(lines).alignment(Alignment::Left).block(
        Block::default()
            .title(" Wi-Fi Password ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Theme::BORDER))
            .style(Style::default().bg(Theme::BG)),
    );
    f.render_widget(paragraph, popup_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn render_speed_test(f: &mut Frame, app: &App, area: Rect) {
    let main_block = Theme::container_block("Network Performance");
    let inner = main_block.inner(area);
    f.render_widget(main_block, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(inner);
    render_cloud_visual(f, app, chunks[0]);
    render_speed_metrics(f, app, chunks[1]);
}

// --- HIGH QUALITY FRACTAL CLOUD VISUALIZATION ---

fn hash_noise(x: f64, y: f64) -> f64 {
    let n = x.sin() * 43758.5453 + y.cos() * 129898.0;
    (n - n.floor()) * 2.0 - 1.0 // Range [-1, 1]
}

fn smooth_interp(a: f64, b: f64, t: f64) -> f64 {
    let ft = t * t * (3.0 - 2.0 * t); // Hermite smoothing
    a + ft * (b - a)
}

fn value_noise(x: f64, y: f64) -> f64 {
    let ix = x.floor();
    let iy = y.floor();
    let fx = x - ix;
    let fy = y - iy;

    let v00 = hash_noise(ix, iy);
    let v10 = hash_noise(ix + 1.0, iy);
    let v01 = hash_noise(ix, iy + 1.0);
    let v11 = hash_noise(ix + 1.0, iy + 1.0);

    let i0 = smooth_interp(v00, v10, fx);
    let i1 = smooth_interp(v01, v11, fx);
    
    smooth_interp(i0, i1, fy)
}

fn fbm(x: f64, y: f64) -> f64 {
    let mut total = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    let mut max_value = 0.0;

    // 4 octaves for balance between detail and performance
    for _ in 0..4 {
        total += value_noise(x * frequency, y * frequency) * amplitude;
        max_value += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    
    total / max_value
}

fn render_cloud_visual(f: &mut Frame, app: &App, area: Rect) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w < 10 || h < 4 {
        return;
    }

    // Animation Speed Logic
    let speed_mult = if app.speed_test.is_running { 1.5 } else { 0.3 };
    let time_offset = (app.anim_frame as f64) * 0.02 * speed_mult;
    
    // Scale factors
    let scale_x = 3.0; 
    let scale_y = 1.5;

    // Safe ASCII/Unicode gradient for density mapping
    // This avoids Braille corruption issues entirely while still looking good
    let density_chars = [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

    let mut lines = Vec::with_capacity(h);

    for y in 0..h {
        let mut spans = Vec::with_capacity(w);
        let ny = (y as f64) / (h as f64);
        
        for x in 0..w {
            let nx = (x as f64) / (w as f64);
            
            // Sample noise field with horizontal scroll
            let sample_x = nx * scale_x + time_offset;
            let sample_y = ny * scale_y;
            
            let mut val = fbm(sample_x, sample_y);
            
            // Normalize [-1, 1] to [0, 1]
            val = (val + 1.0) / 2.0;
            
            // Thresholding to create distinct cloud shapes instead of uniform fog
            if val < 0.45 {
                val = 0.0;
            } else {
                // Sharpen contrast
                val = ((val - 0.45) / 0.55).powf(1.5); 
            }
            
            // Edge Fade
            let edge_dist = (nx.min(1.0 - nx)).min(ny.min(1.0 - ny));
            let fade = (edge_dist * 5.0).clamp(0.0, 1.0);
            val *= fade;

            // Map to char index
            let idx = (val * (density_chars.len() - 1) as f64) as usize;
            let ch = density_chars[idx.min(density_chars.len() - 1)];
            
            // Styling based on density
            let style = if val > 0.7 {
                Style::default().fg(Theme::ACCENT).add_modifier(Modifier::BOLD)
            } else if val > 0.4 {
                Style::default().fg(Theme::PRIMARY)
            } else if val > 0.1 {
                Style::default().fg(Theme::MUTED)
            } else {
                Style::default().fg(Theme::BG)
            };

            spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(spans));
    }
    
    let p = Paragraph::new(lines).alignment(Alignment::Center);
    f.render_widget(p, area);
}

fn render_speed_metrics(f: &mut Frame, app: &App, area: Rect) {
    let ping = app
        .speed_test
        .ping_ms
        .map_or("---".to_string(), |v| format!("{:.1} ms", v));
    let dl = app
        .speed_test
        .download_mbps
        .map_or("---".to_string(), |v| format!("{:.2} Mbps", v));
    let ul = app
        .speed_test
        .upload_mbps
        .map_or("---".to_string(), |v| format!("{:.2} Mbps", v));

    let total_bar_cells: usize = 24;
    let filled_cells = ((app.speed_test.progress_pct as usize) * total_bar_cells) / 100;
    let empty_cells = total_bar_cells.saturating_sub(filled_cells);
    
    let bar_spans = vec![
        Span::styled("  Test Progress: ", Style::default().fg(Theme::PRIMARY)),
        Span::styled("[", Style::default().fg(Theme::BORDER)),
        Span::styled(
            "─".repeat(filled_cells),
            Style::default()
                .fg(Theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " ".repeat(empty_cells),
            Style::default().fg(Theme::HIGHLIGHT),
        ),
        Span::styled(
            format!("] {:>3}%  ", app.speed_test.progress_pct),
            Style::default()
                .fg(Theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("({})", app.speed_test.stage),
            Style::default().fg(Theme::MUTED),
        ),
    ];

    let stats = vec![
        Line::from(""),
        Line::from(bar_spans),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Latency:   ", Style::default().fg(Theme::PRIMARY)),
            Span::styled(
                format!("{:<14}", ping),
                Style::default()
                    .fg(Theme::SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Download:  ", Style::default().fg(Theme::PRIMARY)),
            Span::styled(
                format!("{:<16}", dl),
                Style::default()
                    .fg(Theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Upload:    ", Style::default().fg(Theme::PRIMARY)),
            Span::styled(
                ul,
                Style::default()
                    .fg(Theme::WARNING)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [r] ",
                Style::default()
                    .fg(Theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Restart Test    ", Style::default().fg(Theme::TEXT)),
            Span::styled(
                "[n] / [Esc] ",
                Style::default()
                    .fg(Theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Back to Networks", Style::default().fg(Theme::TEXT)),
        ]),
    ];
    let p = Paragraph::new(stats)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Theme::HIGHLIGHT)),
        )
        .alignment(Alignment::Left);
    f.render_widget(p, area);
}