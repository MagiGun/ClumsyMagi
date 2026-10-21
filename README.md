<div align="center">
  <img src="assets/clumsymagi.svg" width="160" height="160" alt="ClumsyMagi Logo" />
  <h1>ClumsyMagi</h1>
  <p><b>A modern, interactive network condition simulator for Windows.</b></p>
  <p><i>A fork and complete modern rewrite of <a href="https://github.com/jagt/clumsy"><b>clumsy 0.3</b></a> in Rust and egui.</i></p>

  <p>
    <img src="https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011%20(x64)-blue.svg" alt="Platform" />
    <img src="https://img.shields.io/badge/Language-Rust%202021-orange.svg" alt="Rust" />
    <img src="https://img.shields.io/badge/GUI-egui%200.37-green.svg" alt="egui" />
    <img src="https://img.shields.io/badge/License-MIT-purple.svg" alt="License" />
  </p>
</div>

---

## Overview

**ClumsyMagi** is a fork and ground-up modernization of [clumsy 0.3](https://github.com/jagt/clumsy) by [jagt](https://github.com/jagt). It degrades and manipulates live network packets on Windows in a controlled, interactive, and managed manner.

Leveraging the [WinDivert](https://reqrypt.org/windivert.html) packet interception engine, ClumsyMagi intercepts network packets in kernel space, applies customizable network impairments (such as artificial latency, packet loss, bandwidth throttling, packet duplication, out-of-order delivery, TCP connection resets, or packet tampering), and reinjects them into the network stack.

Whether you are debugging multiplayer game netcode, stress-testing distributed microservices, evaluating client-server timeouts, or simulating hostile networking environments, ClumsyMagi provides instant visual and metrics feedback without requiring proxies, network bridges, or code changes to target applications.

---

## Features

ClumsyMagi preserves the functionality of original clumsy while introducing modern usability, process awareness, and graphics capabilities built on Rust and `egui 0.37`:

- **Process and Window Targeting**: Select any running game, application, or top-level window from an interactive dropdown. ClumsyMagi automatically enumerates and live-tracks its active UDP and TCP socket ports, synthesizing targeted capture filters on the fly.
- **One-Click Quick Lag**: Instantly enable outbound latency (default 100 ms) on the targeted process without manually authoring or editing filter syntax.
- **Theme Engine**: Built-in support for **Dark**, **Light**, and **System** themes. System mode dynamically synchronizes with Windows Personalization settings (`AppsUseLightTheme`), backed by hardware-accelerated OpenGL clear-color buffers.
- **Vector UI Graphics**: Clean, resolution-independent vector icons that automatically adapt their rendering color to match the active theme.
- **Display Work-Area Centering**: Automatically calculates the primary display's usable work area (accounting for multi-monitor setups, taskbars, and DPI scaling) so the window opens centered on launch and maintains its position across UAC elevation restarts.
- **Seamless UAC Elevation**: One-click administrator relaunch that seamlessly transfers filter parameters, presets, and window coordinates.
- **Realtime Throughput Graph and Event Log**: Live activity charts (captured vs. injected KB/s) and scrolling packet audit logs with clipboard export.
- **Console Window Toggle**: Runs as a clean GUI application by default, with an optional in-app toggle to attach or detach the Windows command prompt for log inspection.
- **Configuration Persistence**: Presets, UI preferences, and custom filters are automatically stored as JSON in `%APPDATA%\ClumsyMagi\config.json`.

---

## Network Impairment Modules

All eight core network impairment modules from clumsy 0.3 are implemented and optimized:

| Module | Description | Key Parameters |
| :--- | :--- | :--- |
| **Lag** | Holds packets in a precision buffer and releases them after a delay | Inbound / Outbound direction, Delay (ms) |
| **Drop** | Randomly discards packets based on probability | Chance (0% – 100%) |
| **Throttle** | Batches packets and transmits them only on periodic timeframe ticks | Timeframe (ms), Drop on Throttle |
| **Duplicate** | Generates additional cloned copies of intercepted packets | Chance (%), Copy count (1 – 5) |
| **Out of Order** | Swaps pairs of consecutive packets to disrupt packet ordering | Chance (%) |
| **Tamper** | Inverts or mutates payload bits with optional checksum correction | Chance (%), Checksum recomputation |
| **Reset** | Injects TCP RST flags to forcibly terminate active connections | Chance (%) |
| **Bandwidth** | Enforces a strict bandwidth ceiling (KB/s), dropping excess data | Limit (KB/s) |

---

## Getting Started

### Prerequisites

- **Operating System**: Windows 10 or Windows 11 (64-bit recommended).
- **Administrator Privileges**: Administrator access is required by WinDivert to install its kernel packet filtering driver. If launched as a standard user, use the in-app **Elevate** button to restart with elevated privileges.

### Running Prebuilt Binaries

1. Download the release archive from GitHub Releases or compile from source.
2. Ensure `WinDivert.dll` and `WinDivert64.sys` are located in the same directory as `clumsymagi.exe`.
3. Right-click `clumsymagi.exe` and select **Run as Administrator** (or launch normally and click **Elevate**).

---

## Command Line Interface and Automation

ClumsyMagi can be driven from the command line for scripted testing and automated CI pipelines:

```bash
# Apply 150 ms outbound latency on UDP packets and start filtering immediately
clumsymagi --filter "udp and outbound" --lag on --lag-time 150 --auto-start

# Introduce 10% packet drop and shut down after 60 seconds
clumsymagi --filter "outbound" --drop on --drop-chance 10 --quit-after 60

# Keep the Windows command prompt visible during execution
clumsymagi --show-console

# Specify initial window placement
clumsymagi --window-pos 400,200
```

### Command Line Options

| Option | Description |
| :--- | :--- |
| `--filter <EXPR>` | WinDivert filter expression (e.g. `"udp and outbound"`, `"tcp.DstPort == 80"`) |
| `--auto-start` | Starts filtering immediately upon application launch |
| `--show-console` | Keeps the Windows console window attached and visible |
| `--quit-after <SECS>` | Automatically shuts down after the specified number of seconds |
| `--theme <dark\|light\|system>` | Overrides the application color theme |
| `--window-pos <X,Y>` | Sets the initial window coordinates on screen |
| `--lag on\|off` | Enables or disables the Lag module |
| `--lag-time <MS>` | Configures latency delay in milliseconds |
| `--drop on\|off` | Enables or disables the Drop module |
| `--drop-chance <%>` | Drop probability percentage |
| `--throttle on\|off` | Enables or disables Throttling |
| `--throttle-frame <MS>` | Throttle tick window in milliseconds |
| `--throttle-drop on\|off` | Drops throttled packets instead of queuing |
| `--duplicate on\|off` | Enables packet duplication |
| `--duplicate-count <N>` | Number of duplicates to send |
| `--ood on\|off` | Enables Out of Order delivery |
| `--tamper on\|off` | Enables packet tampering |
| `--tamper-checksum on\|off` | Recomputes IP/TCP/UDP checksums after tampering |
| `--reset on\|off` | Enables TCP Reset injection |
| `--bandwidth-limit <KBPS>` | Limits throughput to specified KB/s |
| `--selftest` | Executes internal headless selftest suite and exits |

---

## Building from Source

To compile ClumsyMagi from source:

1. Install the latest stable [Rust toolchain](https://www.rust-lang.org/tools/install):
   ```bash
   rustup default stable-x86_64-pc-windows-msvc
   ```
2. Clone the repository:
   ```bash
   git clone https://github.com/uvxszi/ClumsyMagi.git
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
5. The compiled executable `clumsymagi.exe` along with `WinDivert.dll` and `WinDivert64.sys` will be generated in `rust/target/release/`.

---

## CI/CD and Release Automation

ClumsyMagi includes pre-configured GitHub Actions workflows in `.github/workflows/`:

- **CI (`ci.yml`)**: Automatically runs code checks and unit tests on pull requests and commits to `master` / `main`.
- **Release (`release.yml`)**: Builds the optimized Windows 64-bit release, embeds the application icon and resource metadata into `clumsymagi.exe`, and packages `clumsymagi.exe`, `WinDivert.dll`, `WinDivert64.sys`, `README.md`, and `LICENSE` into `clumsymagi-windows-x86_64.zip`.
  - **Manual Trigger**: Open **Actions** → **Release** → **Run workflow** in GitHub. You can optionally specify a release tag (e.g., `v0.4.0`) to publish a GitHub Release or leave it blank to generate the downloadable artifact.
  - **Tag Trigger**: Pushing a tag prefixed with `v` (e.g. `git tag v0.4.0 && git push origin v0.4.0`) automatically builds, packages, and publishes a new GitHub Release with binary assets.

---

## WinDivert Filter Reference

WinDivert filters follow standard packet filtering syntax:

- **Protocols**: `tcp`, `udp`, `icmp`, `icmpv6`, `ip`, `ipv6`
- **Direction**: `inbound`, `outbound`
- **Interface**: `loopback`
- **Fields**: `tcp.DstPort == 80`, `udp.SrcPort == 53`, `ip.DstAddr == 1.1.1.1`
- **Operators**: `and`, `or`, `not`, `==`, `!=`, `<`, `>`, `<=`, `>=`

*Example*: Filter all outbound UDP traffic destined for port 7777:
```text
udp and outbound and udp.DstPort == 7777
```

---

## License and Acknowledgments

- **ClumsyMagi**: Distributed under the terms of the [MIT License](LICENSE).
- **clumsy**: ClumsyMagi is a fork and modernization of [clumsy 0.3](https://github.com/jagt/clumsy) originally developed by [jagt](https://github.com/jagt), licensed under the MIT License.
- **WinDivert**: Uses [WinDivert](https://reqrypt.org/windivert.html) by basil, distributed under the GNU Lesser General Public License (LGPLv3) or GNU General Public License (GPLv2).
