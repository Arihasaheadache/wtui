# WTUI

WTUI is a Rust based TUI for NetworkManager. It hopes to replace the old archaic looking TUI and replace it with a cleaner, modern interface.

The interface is built with modern Wayland compositors in mind, and works well with transparency and blur effects as well. It is designed to inherit your terminal design to be in unison with your system (in case of ricing).

## Features

- **Direct NetworkManager D-Bus Integration:** Communicates directly over the system D-Bus using `zbus` for high performance and low latency without brittle shell command parsing.
- **Dual Wired & Wireless Support:** Introspects both Ethernet interfaces (link speed detection) and 802.11 Wi-Fi access points.
- **Dynamic Wi-Fi Signal Meters:** Visual signal strength indicators (`▂___`, `▂▄__`, `▂▄▆_`, `▂▄▆█`) along with percentage ratings.
- **Interactive Mobile QR Code Sharing:**
  - Automatically formats standard Wi-Fi configuration strings.
  - Renders pixel-crisp QR codes directly in your terminal.
  - Securely prompts for connection secrets via D-Bus secrets API or an ephemeral credential prompt fallback.
- **One-Key Connection & Profile Management:**
  - Connect / Disconnect active network devices.
  - Toggle autoconnect flags across saved NetworkManager connection profiles.
  - Trigger active wireless scans on demand.
- **Aesthetic UI:** Styled using the Catppuccin Mocha palette with rounded container borders and responsive split-pane layouts.

## Controls

| Key       | Action                                      |
| --------- | ------------------------------------------- |
| `j` / `↓` | Move selection down                         |
| `k` / `↑` | Move selection up                           |
| `c`       | Connect / Disconnect selected network       |
| `a`       | Toggle connection autoconnect state         |
| `s`       | Toggle mobile Wi-Fi sharing QR code pane    |
| `n`       | Toggle Internet Speed Test tab              |
| `r`       | Request a fresh Wi-Fi rescan                |
| `q` / `Esc` | Exit application                          |

## Requirements

- **NetworkManager:** This project strictly targets NetworkManager. It will not work with `iwctl` or other network backends.
- **Rust Toolchain:** Rust 1.70+ recommended via [rustup](https://rustup.rs/).
- **D-Bus & Polkit Permissions:** Required to view and manipulate connections, activate/deactivate networks, and retrieve network secrets.

## Permissions / Polkit

On some systems, Polkit may require root privileges for NetworkManager operations such as connecting to new networks, disconnecting devices, modifying saved connection profiles, or retrieving Wi-Fi secrets.

If WTUI fails to connect or modify connections as your normal user, you have two options.

### Option 1: Run WTUI with `sudo`

You can temporarily run WTUI as root:

```bash
sudo wtui
```

If the binary was installed into your user-local bin directory and root's `PATH` does not include it, run it with the full path:

```bash
sudo ~/.local/bin/wtui
```

Or, if running from a local build:

```bash
sudo ./target/release/wtui
```

### Option 2: Grant your user NetworkManager permissions via Polkit

To allow your normal user to manage NetworkManager without running WTUI as root, create a Polkit rule.

Run the following command from your normal user account:

```bash
sudo tee /etc/polkit-1/rules.d/50-org.freedesktop.NetworkManager.rules << EOF
polkit.addRule(function(action, subject) {
    if (action.id.indexOf("org.freedesktop.NetworkManager.") == 0 &&
        subject.user == "$(whoami)") {
        return polkit.Result.YES;
    }
});
EOF
```

Then restart the Polkit service:

```bash
sudo systemctl restart polkit
```

If the change does not take effect immediately, reboot your system.

> **Note:** This rule grants the current user full permission to manage NetworkManager connections without an authentication prompt. This is usually desirable on personal machines, but avoid using it on shared or security-sensitive systems unless you understand the implications.

## Installation & Build

Clone the repository:

```bash
git clone https://github.com/Arihasaheadache/wtui.git
cd wtui
```

Build from source:

```bash
cargo build --release
```

Install binary to system path:

```bash
install -Dm755 target/release/wtui ~/.local/bin/wtui
```

Or install with Cargo:

```bash
cargo install --path .
```

## Preview

Images blurred for privacy.

### Interface

![Interface](./assets/simple.png)

### QR Section

![QR Section](./assets/qr.png)

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).