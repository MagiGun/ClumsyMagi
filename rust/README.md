# ClumsyMagi

**ClumsyMagi** is a modern, pure Rust + [egui](https://github.com/emilk/egui) remake of [clumsy 0.3](https://github.com/jagt/clumsy), designed to simulate poor network conditions (latency, packet loss, bandwidth throttling, duplication, tampering, and out-of-order delivery) interactively and in a controlled manner on Windows.

---

## Highlights & Features

- **Modern UI powered by egui/eframe 0.37**:
  - Dark, Light, and System themes (hot-toggleable).
  - Responsive, high-performance rendering.
  - Live packet throughput plot (Captured/s vs Sent/s).
  - Live stats banner: Captured, Sent, Dropped, Modified, Send Failed counters.
  - Live activity log with timestamped entries, copy-to-clipboard, and clear functionality.
  - Real-time activity indicator dots (green = active traffic, red = send failure, gray = idle) per module and globally.
- **Full Clumsy 0.3 Feature Parity**:
  - **Lag**: Delays packets (0–15,000 ms) with direction filtering (inbound/outbound/both) and ring-buffer overflow safety.
  - **Drop**: Drops packets with configurable percentage chance (0–100%).
  - **Throttle**: Holds packets over a timeframe (0–1,000 ms), then flushes or drops them together.
  - **Duplicate**: Clones matching packets (2–50 copies) with percentage chance.
  - **Out of order**: Reorders packets, swapping adjacent pairs or holding single packets until the next arrives.
  - **Tamper**: Mutates payload bytes while protecting protocol headers, with optional checksum recalculation.
  - **Set TCP RST**: Injects TCP RST flags into TCP connections on demand or based on chance, complete with checksum updates.
  - **Bandwidth**: Enforces rate limiting (KB/s) with a 1-second sliding window, dropping excess packets.
- **Presets & Configuration**:
  - Includes all built-in clumsy presets (localhost ipv4 loopback, all outbound/inbound, specific IP, ports, IPv6, ICMP).
  - Automatically loads and merges presets from `config.txt` if present next to the executable.
  - Custom user presets and parameter preferences are persisted across runs in `%APPDATA%/ClumsyMagi/config.json`.
- **Live Filter Syntax Validation**:
  - Compiles filter expressions using WinDivert's engine in real time with error diagnostics and cursor error positions before capturing starts.
- **Elevation Detection & Seamless Relaunch**:
  - Detects if running without administrator privileges.
  - Includes a one-click **"🛡 Restart as Administrator"** button using standard Windows UAC (`runas`), preserving configuration across the relaunch.
- **Dynamic WinDivert 2.2 FFI**:
  - Loads `WinDivert.dll` dynamically at runtime, providing clear, actionable error messages if drivers or DLLs are missing.
  - Safe RAII management for device handles, multithreaded capture, and timer resolution periods (`timeBeginPeriod`).
- **Comprehensive CLI**:
  - Run headless or automated scripted tests using `--filter`, `--auto-start`, `--quit-after SECONDS`, and module switches (`--lag on --lag-time 200`, etc.).
  - Built-in `--selftest` diagnostic flag to verify DLL loading, filter compiler, and address marshalling.

---

## Building

### Prerequisites

- **Windows 10 / 11** (x64)
- **Rust toolchain** (1.80+ or latest stable)

### Build Commands

```powershell
# Navigate to the rust directory
cd rust

# Build debug binary and run tests
cargo test

# Build optimized release binary
cargo build --release
```

The build script automatically copies `WinDivert.dll`, `WinDivert64.sys`, and `config.txt` directly to `target/release/` or `target/debug/`.

---

## Running

### GUI Mode

To capture system-wide network packets, ClumsyMagi must be run with Administrator privileges (required by the WinDivert driver):

```powershell
# Run from an elevated PowerShell / Command Prompt:
.\target\release\clumsymagi.exe
```

If launched from a normal user prompt, ClumsyMagi will open with a prominent **"🛡 Restart as Administrator"** button.

### Command-Line Arguments

ClumsyMagi supports all classic clumsy arguments as well as modern extensions:

```text
Usage: clumsy [options]

Options:
  --filter FILTER              Set capture filter (e.g. "udp and outbound")
  --auto-start                 Start filtering immediately on launch
  --quit-after SECONDS         Exit after SECONDS (useful for automated testing)
  --theme dark|light|system    UI theme override
  --selftest                   Run self-checks and exit

Module switches:
  --lag on|off                 Enable/disable Lag
  --lag-inbound on|off         Apply Lag to inbound packets
  --lag-outbound on|off        Apply Lag to outbound packets
  --lag-time MS                Delay in milliseconds (0-15000)

  --drop on|off                Enable/disable Drop
  --drop-chance PERCENT        Drop probability percentage (e.g. 15.0)

  --throttle on|off            Enable/disable Throttle
  --throttle-frame MS          Throttle timeframe in milliseconds
  --throttle-drop on|off       Drop throttled packets instead of sending

  --duplicate on|off           Enable/disable Duplicate
  --duplicate-count COUNT      Number of clones to send (2-50)
  --duplicate-chance PERCENT   Duplication probability percentage

  --ood on|off                 Enable/disable Out of Order
  --ood-chance PERCENT         Reorder probability percentage

  --tamper on|off              Enable/disable Tamper
  --tamper-chance PERCENT      Tamper probability percentage
  --tamper-checksum on|off     Recompute checksums after tampering

  --reset on|off               Enable/disable TCP RST
  --reset-chance PERCENT       RST probability percentage

  --bandwidth on|off           Enable/disable Bandwidth Cap
  --bandwidth-limit KBPS       Bandwidth cap in KB/s
```

#### Examples

Simulate 150ms outbound lag on UDP:
```powershell
.\target\release\clumsymagi.exe --filter "udp and outbound" --lag on --lag-time 150 --auto-start
```

Run a 10-second automated test dropping 20% of TCP traffic:
```powershell
.\target\release\clumsymagi.exe --filter "tcp" --drop on --drop-chance 20 --auto-start --quit-after 10
```

---

## Architecture

- **`main.rs`**: Application entry point, CLI parsing, single instance enforcement, and eframe initialization.
- **`app.rs`**: egui GUI layout (filter controls, presets, module cards, throughput plot, event log, elevation banners).
- **`engine/`**:
  - `mod.rs`: Capture and processing engine with dedicated receiver and 10ms clock threads.
  - `modules.rs`: Core packet degradation logic for Lag, Drop, Throttle, Duplicate, OOD, Tamper, Reset, and Bandwidth.
  - `params.rs`: Lock-free atomic parameters shared between UI and packet threads.
  - `rng.rs`: High-performance pseudo-random number generator.
- **`packet.rs`**: Pure-Rust IPv4, IPv6, TCP, UDP, and ICMP/ICMPv6 parser, checksum recalculator, and TCP flag manipulator.
- **`wd.rs`**: Dynamic loader and safe wrapper around `WinDivert.dll` and `WINDIVERT_ADDRESS`.
- **`platform.rs`**: Windows platform helpers (UAC elevation check & relaunch, global single-instance mutex, multimedia timer resolution).
- **`config.rs`**: Preset configuration management and JSON persistence.
- **`selftest.rs`**: Diagnostics for CI/first-run verification.
