<div align="center">
  <img src="assets/clumsymagi.svg" width="160" height="160" alt="ClumsyMagi Logo" />
  <h1>ClumsyMagi</h1>
  <p><b>A modern, interactive network condition simulator for Windows.</b></p>
  <p><i>The complete modern rewrite of the classic <b>clumsy 0.3</b> in Rust and egui.</i></p>

  <p>
    <img src="https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011%20(x64)-blue.svg" alt="Platform" />
    <img src="https://img.shields.io/badge/Language-Rust%202021-orange.svg" alt="Rust" />
    <img src="https://img.shields.io/badge/GUI-egui%200.37-green.svg" alt="egui" />
    <img src="https://img.shields.io/badge/License-MIT-purple.svg" alt="License" />
  </p>
</div>

---

## 🧙‍♂️ Overview

**ClumsyMagi** degrades and manipulates live network packets on Windows in a controlled, interactive, and highly managed manner. Leveraging the powerful [WinDivert](https://reqrypt.org/windivert.html) engine, it intercepts living network packets on demand, applies customizable impairments (such as artificial lag, packet loss, bandwidth throttling, packet duplication, out-of-order delivery, TCP resets, or bit corruption), and reinjects them back into the network stack.

Whether you are debugging multiplayer netcode, stress-testing distributed microservices, evaluating client-server timeouts, or simulating hostile networking environments, ClumsyMagi provides an interactive, instant feedback loop without requiring proxies, network bridges, or code changes in your applications.

---

## ✨ Modern Features

ClumsyMagi is a ground-up reimagining of clumsy built in Rust with immediate-mode GUI (`egui 0.37`):

- **🎮 Direct Game & Process Targeting**: Select any running game, application, or window. ClumsyMagi automatically enumerates and live-tracks its active UDP and TCP socket ports, generating targeted filters on the fly.
- **⚡ 1-Click Quick Lag**: Instantly configure outbound packet lag (100ms) on any chosen game without having to write or edit complex WinDivert filter strings.
- **🌙 Full Theme Support**: Built-in support for **Dark**, **Light**, and **System** themes (automatically synced with your Windows Personalization settings), complete with proper clear-color OpenGL backbuffers.
- **🎨 Vector Icons**: All emojis and icons have been replaced by crisp, resolution-independent vector graphics that automatically dynamically shift between pure white in Dark mode and pure black in Light mode.
- **🎯 Work-Area Screen Centering**: Automatically calculates the primary/active display's usable work area (accounting for multi-monitors, taskbars, and DPI scaling) so the window always spawns centered on screen.
- **🛡 Seamless UAC Elevation**: One-click administrator relaunch that preserves your exact window position on screen.
- **📈 Realtime Throughput Graph & Event Log**: Live activity graphs (captured vs. sent KB/s) and scrolling packet audit logs with one-click clipboard export.
- **📟 Embedded Console Management**: Runs as a clean desktop GUI by default with an optional interactive toggle for the Windows command prompt.
- **💾 Fast Config Persistence**: User-created presets and UI preferences are automatically saved as JSON in `%APPDATA%\ClumsyMagi\config.json`.

---

## 🧰 Network Impairment Modules

All eight core network modules from clumsy have been faithfully ported and optimized:

| Module | Description | Key Parameters |
| :--- | :--- | :--- |
| **Lag** | Holds packets in a precision buffer and releases them after a delay | Inbound / Outbound, Delay (ms) |
| **Drop** | Randomly discards packets based on a probability dice roll | Chance (0% – 100%) |
| **Throttle** | Batches packets and transmits them only on periodic timeframe ticks | Timeframe (ms), Drop on Throttle |
| **Duplicate** | Generates additional cloned copies of intercepted packets | Chance (%), Copy count (1 – 5) |
| **Out of Order** | Swaps pairs of consecutive packets to disrupt packet ordering | Chance (%) |
| **Tamper** | Inverts or mutates payload bits with optional checksum correction | Chance (%), Checksum recompute |
| **Reset** | Injects TCP RST flags to forcibly terminate active TCP connections | Chance (%) |
| **Bandwidth** | Enforces a strict bandwidth ceiling (KB/s), dropping excess data | Limit (KB/s) |

---

## 🚀 Getting Started

### Prerequisites

- **OS**: Windows 10 or Windows 11 (64-bit recommended)
- **Privileges**: Administrator rights are required by WinDivert to install its kernel packet filter driver. If launched without admin rights, ClumsyMagi features an **Elevate** button to restart elevated automatically.

### Running Prebuilt Binary

1. Download or compile `clumsymagi.exe`.
2. Ensure `WinDivert.dll` and `WinDivert64.sys` reside in the same folder as `clumsymagi.exe` (or in the system path).
3. Right-click `clumsymagi.exe` and select **Run as Administrator** (or click the in-app **Elevate** button).

---

## 🛠 Command Line & Automation

ClumsyMagi can be launched in scripted workflows and automated testing runs using CLI arguments:

```bash
# Basic outbound UDP lag of 150ms with auto-start
clumsymagi --filter "udp and outbound" --lag on --lag-time 150 --auto-start

# Introduce 10% packet drop and exit after 60 seconds
clumsymagi --filter "outbound" --drop on --drop-chance 10 --quit-after 60

# Keep the Windows command prompt visible
clumsymagi --show-console

# Specify initial window position
clumsymagi --window-pos 400,200
```

### Full CLI Options

| Argument | Description |
| :--- | :--- |
| `--filter <EXPR>` | Capture filter expression (e.g. `"udp and outbound"`, `"tcp.DstPort == 80"`) |
| `--auto-start` | Starts filtering immediately upon launch |
| `--show-console` | Keeps the Windows console/cmd window attached and visible |
| `--quit-after <SECS>` | Automatically shuts down after the specified number of seconds |
| `--theme <dark\|light\|system>` | Overrides the application color theme |
| `--window-pos <X,Y>` | Sets the initial window coordinates on screen |
| `--lag on\|off` | Enables or disables the Lag module |
| `--lag-time <MS>` | Configures delay in milliseconds |
| `--drop on\|off` | Enables or disables the Drop module |
| `--drop-chance <%>` | Drop trigger probability (e.g. `12.5`) |
| `--throttle on\|off` | Enables or disables Throttling |
| `--throttle-frame <MS>` | Throttle tick window in milliseconds |
| `--throttle-drop on\|off` | Drop throttled packets instead of queuing |
| `--duplicate on\|off` | Enables packet duplication |
| `--duplicate-count <N>` | Number of duplicates to send |
| `--ood on\|off` | Enables Out of Order delivery |
| `--tamper on\|off` | Enables packet tampering |
| `--tamper-checksum on\|off` | Recomputes IP/TCP/UDP checksums after tampering |
| `--reset on\|off` | Enables TCP Reset injection |
| `--bandwidth-limit <KBPS>` | Limits throughput to specified KB/s |
| `--selftest` | Executes internal headless selftest and exits |

---

## 🏗 Building from Source

To build ClumsyMagi from source:

1. Install the latest stable [Rust toolchain](https://www.rust-lang.org/tools/install):
   ```bash
   rustup default stable-x86_64-pc-windows-msvc
   ```
2. Clone the repository:
   ```bash
   git clone https://github.com/your-username/ClumsyMagi.git
   cd ClumsyMagi/rust
   ```
3. Run the unit test suite:
   ```bash
   cargo test
   ```
4. Build the optimized release binary:
   ```bash
   cargo build --release
   ```
5. The compiled executable `clumsymagi.exe` alongside `WinDivert.dll` and `WinDivert64.sys` will be generated in `rust/target/release/`.

---

## 🚀 GitHub Actions & Automated Releases

ClumsyMagi includes pre-configured GitHub Actions workflows in `.github/workflows/`:

- **CI (`ci.yml`)**: Automatically runs code checks and the full test suite on pushes and pull requests to `master` / `main`.
- **Release (`release.yml`)**: Builds the optimized Windows 64-bit release, embeds the application icon, packages `clumsymagi.exe`, `WinDivert.dll`, `WinDivert64.sys`, `README.md`, and `LICENSE` into a `clumsymagi-windows-x86_64.zip` archive.
  - **Manual Trigger**: Go to **Actions** → **Release** → **Run workflow**. You can optionally specify a release tag (e.g., `v0.4.0`) to publish a GitHub Release or leave it blank to build and upload the zipped artifact.
  - **Tag Trigger**: Pushing any tag starting with `v` (e.g. `git tag v0.4.0 && git push origin v0.4.0`) automatically builds, packages, and publishes a new GitHub Release with attached binaries.

---

## 📜 WinDivert Filter Reference

WinDivert filters use a simple syntax:

- **Protocols**: `tcp`, `udp`, `icmp`, `icmpv6`, `ip`, `ipv6`
- **Direction**: `inbound`, `outbound`
- **Interface**: `loopback`
- **Fields**: `tcp.DstPort == 80`, `udp.SrcPort == 53`, `ip.DstAddr == 1.1.1.1`
- **Operators**: `and`, `or`, `not`, `==`, `!=`, `<`, `>`, `<=`, `>=`

*Example*: Filter all outbound UDP packets going to port 7777:
```text
udp and outbound and udp.DstPort == 7777
```

---

## 📄 License

ClumsyMagi is distributed under the terms of the **MIT License**. See [LICENSE](LICENSE) for details.
WinDivert is distributed under the GNU Lesser General Public License (LGPL) or GNU General Public License (GPL).
