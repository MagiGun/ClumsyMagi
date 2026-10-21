//! Command line interface, in the spirit of clumsy's `--key value` arguments:
//! `clumsymagi --filter "udp and outbound" --lag on --lag-time 200 --auto-start`
//! Values are plain words (shell handles quoting). `on/off` accepted for
//! booleans, numbers for numerics, percents like `12.5` for chances.

use crate::config::AppConfig;

#[derive(Debug, Clone, Default)]
pub struct CliOptions {
    pub auto_start: bool,
    pub quit_after_secs: Option<u64>,
    /// internal flag for the UAC relaunch path (retry single-instance)
    pub elevated_relaunch: bool,
    /// Keep console window visible instead of hiding it by default
    pub show_console: bool,
    /// Specific initial window position requested, e.g. from elevated relaunch
    pub window_pos: Option<[f32; 2]>,
}

pub fn help_text() -> String {
    let mut s = String::new();
    s.push_str("ClumsyMagi - makes your network condition on Windows significantly worse,\n");
    s.push_str("in a managed and interactive manner.\n\n");
    s.push_str("Usage: clumsy [options]\n\n");
    s.push_str("Options are given as `--key value` pairs:\n\n");
    s.push_str("  --filter FILTER        set the capture filter\n");
    s.push_str("  --auto-start           start filtering immediately\n");
    s.push_str("  --show-console         keep the console/cmd window visible\n");
    s.push_str("  --quit-after SECONDS   exit after SECONDS (handy for scripted runs)\n\n");
    for kind in crate::engine::all_modules() {
        let n = kind.short_name();
        s.push_str(&format!("  --{n} on|off             enable/disable {n}\n"));
        s.push_str(&format!("  --{n}-inbound on|off    process inbound packets\n"));
        s.push_str(&format!("  --{n}-outbound on|off   process outbound packets\n"));
        match kind {
            crate::engine::modules::ModuleKind::Lag => {
                s.push_str(&format!("  --{n}-time MS          delay in ms\n"));
            }
            crate::engine::modules::ModuleKind::Drop
            | crate::engine::modules::ModuleKind::Throttle
            | crate::engine::modules::ModuleKind::Duplicate
            | crate::engine::modules::ModuleKind::Ood
            | crate::engine::modules::ModuleKind::Tamper
            | crate::engine::modules::ModuleKind::Reset => {
                s.push_str(&format!("  --{n}-chance PERCENT   trigger chance, e.g. 12.5\n"));
            }
            _ => {}
        }
    }
    s.push_str("\nModule specific extras:\n");
    s.push_str("  --throttle-frame MS          throttle time frame\n");
    s.push_str("  --throttle-drop on|off       drop throttled packets instead of sending\n");
    s.push_str("  --duplicate-count N          number of copies\n");
    s.push_str("  --tamper-checksum on|off     recompute checksums after tampering\n");
    s.push_str("  --bandwidth-limit KBPS       bandwidth cap in KB/s\n\n");
    s.push_str("  --selftest                   run runtime self checks and exit\n");
    s
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Some(true),
        "off" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_num<T: std::str::FromStr>(v: &str) -> Option<T> {
    v.parse().ok()
}

/// Apply CLI arguments on top of the loaded config. Returns options, or Err
/// with a usage message.
pub fn parse(args: &[String], cfg: &mut AppConfig) -> Result<CliOptions, String> {
    let mut opts = CliOptions::default();
    let mut ix = 0;
    while ix < args.len() {
        let key = &args[ix];
        if key == "--help" || key == "-h" {
            return Err(help_text());
        }
        if !key.starts_with("--") || key.len() <= 2 {
            return Err(format!("invalid argument: {key}. use options like \"--lag on\"; see --help"));
        }
        let key = key[2..].to_string();
        if key == "selftest" {
            opts.auto_start = false;
            ix += 1;
            continue;
        }
        if key == "auto-start" {
            if let Some(val) = args.get(ix + 1) {
                if let Some(b) = parse_bool(val) {
                    opts.auto_start = b;
                    ix += 2;
                    continue;
                }
            }
            opts.auto_start = true;
            ix += 1;
            continue;
        }
        if key == "elevated-relaunch" {
            if let Some(val) = args.get(ix + 1) {
                if let Some(b) = parse_bool(val) {
                    opts.elevated_relaunch = b;
                    ix += 2;
                    continue;
                }
            }
            opts.elevated_relaunch = true;
            ix += 1;
            continue;
        }
        if key == "show-console" {
            if let Some(val) = args.get(ix + 1) {
                if let Some(b) = parse_bool(val) {
                    opts.show_console = b;
                    ix += 2;
                    continue;
                }
            }
            opts.show_console = true;
            ix += 1;
            continue;
        }

        // Allow standalone module toggles: e.g. `--lag`, `--drop` (defaults to on)
        // or `--lag on|off`
        if let Some(kind) = crate::engine::modules::ModuleKind::from_short_name(&key) {
            let mut on = true;
            if let Some(next) = args.get(ix + 1) {
                if let Some(b) = parse_bool(next) {
                    on = b;
                    ix += 2;
                    module_cfg_mut(cfg, kind).enabled = on;
                    continue;
                }
            }
            module_cfg_mut(cfg, kind).enabled = on;
            ix += 1;
            continue;
        }

        let Some(value) = args.get(ix + 1) else {
            return Err(format!("missing value for --{key}; see --help"));
        };
        ix += 2;

        match key.as_str() {
            "filter" => cfg.filter = value.clone(),
            "quit-after" => {
                opts.quit_after_secs = parse_num::<u64>(value).filter(|&v| v > 0);
            }
            "window-pos" => {
                let parts: Vec<&str> = value.split(',').collect();
                if parts.len() == 2 {
                    if let (Ok(x), Ok(y)) = (parts[0].trim().parse::<f32>(), parts[1].trim().parse::<f32>()) {
                        opts.window_pos = Some([x, y]);
                    }
                }
            }
            "theme" => {
                cfg.theme = match value.to_ascii_lowercase().as_str() {
                    "dark" => crate::config::Theme::Dark,
                    "light" => crate::config::Theme::Light,
                    "system" => crate::config::Theme::System,
                    _ => return Err(format!("invalid theme: {value} (dark|light|system)")),
                };
            }
            _ => {
                apply_module_arg(&key, value, cfg)
                    .map_err(|e| format!("--{key}: {e} (see --help)"))?;
            }
        }
    }
    Ok(opts)
}

fn module_cfg_mut(cfg: &mut AppConfig, kind: crate::engine::modules::ModuleKind) -> &mut crate::config::ModuleConfig {
    let name = kind.short_name().to_string();
    cfg.modules.entry(name).or_default()
}

fn apply_module_arg(key: &str, value: &str, cfg: &mut AppConfig) -> Result<(), String> {
    use crate::engine::modules::ModuleKind;
    // keys look like: lag, lag-time, drop-chance, throttle-drop, ...
    for kind in crate::engine::all_modules() {
        let prefix = format!("{}-", kind.short_name());
        if key == kind.short_name() {
            let on = parse_bool(value).ok_or_else(|| format!("expected on|off, got \"{value}\""))?;
            module_cfg_mut(cfg, *kind).enabled = on;
            return Ok(());
        }
        if let Some(suffix) = key.strip_prefix(&prefix) {
            let cfg_m = module_cfg_mut(cfg, *kind);
            match suffix {
                "inbound" => {
                    cfg_m.inbound =
                        parse_bool(value).ok_or_else(|| format!("expected on|off, got \"{value}\""))?;
                    return Ok(());
                }
                "outbound" => {
                    cfg_m.outbound =
                        parse_bool(value).ok_or_else(|| format!("expected on|off, got \"{value}\""))?;
                    return Ok(());
                }
                "chance" => {
                    let pct: f32 = parse_num(value)
                        .ok_or_else(|| format!("expected percent like 12.5, got \"{value}\""))?;
                    if !(0.0..=100.0).contains(&pct) {
                        return Err("chance must be within 0..100".into());
                    }
                    cfg_m.chance_percent = pct;
                    return Ok(());
                }
                "time" if *kind == ModuleKind::Lag => {
                    cfg_m.delay_ms = Some(parse_num::<i32>(value).filter(|&v| (0..=15000).contains(&v)).ok_or_else(
                        || format!("expected ms within 0..15000, got \"{value}\""),
                    )?);
                    return Ok(());
                }
                "frame" if *kind == ModuleKind::Throttle => {
                    cfg_m.throttle_frame_ms =
                        Some(parse_num::<i32>(value).filter(|&v| (0..=1000).contains(&v)).ok_or_else(
                            || format!("expected ms within 0..1000, got \"{value}\""),
                        )?);
                    return Ok(());
                }
                "drop" if *kind == ModuleKind::Throttle => {
                    cfg_m.drop_throttled = Some(
                        parse_bool(value).ok_or_else(|| format!("expected on|off, got \"{value}\""))?,
                    );
                    return Ok(());
                }
                "count" if *kind == ModuleKind::Duplicate => {
                    cfg_m.dup_count = Some(
                        parse_num::<i32>(value)
                            .filter(|&v| (2..=50).contains(&v))
                            .ok_or_else(|| format!("expected count within 2..50, got \"{value}\""))?,
                    );
                    return Ok(());
                }
                "checksum" if *kind == ModuleKind::Tamper => {
                    cfg_m.redo_checksum = Some(
                        parse_bool(value).ok_or_else(|| format!("expected on|off, got \"{value}\""))?,
                    );
                    return Ok(());
                }
                "limit" if *kind == ModuleKind::Bandwidth => {
                    cfg_m.bandwidth_limit_kbps = Some(
                        parse_num::<i32>(value)
                            .filter(|&v| (0..=99999).contains(&v))
                            .ok_or_else(|| format!("expected KB/s within 0..99999, got \"{value}\""))?,
                    );
                    return Ok(());
                }
                _ => {}
            }
        }
    }
    Err(format!("unknown option --{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_module_args() {
        let mut cfg = AppConfig::default();
        let args: Vec<String> = [
            "--lag", "on",
            "--lag-time", "120",
            "--drop", "off",
            "--drop-chance", "12.5",
            "--throttle-frame", "50",
            "--bandwidth-limit", "100",
            "--filter", "udp and outbound",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let opts = parse(&args, &mut cfg).unwrap();
        assert!(!opts.auto_start);
        assert_eq!(cfg.filter, "udp and outbound");
        let lag = cfg.module_config(crate::engine::modules::ModuleKind::Lag);
        assert!(lag.enabled);
        assert_eq!(lag.delay_ms, Some(120));
        let drop = cfg.module_config(crate::engine::modules::ModuleKind::Drop);
        assert!(!drop.enabled);
        assert!((drop.chance_percent - 12.5).abs() < 1e-5);
        let throttle = cfg.module_config(crate::engine::modules::ModuleKind::Throttle);
        assert_eq!(throttle.throttle_frame_ms, Some(50));
        let bw = cfg.module_config(crate::engine::modules::ModuleKind::Bandwidth);
        assert_eq!(bw.bandwidth_limit_kbps, Some(100));
    }

    #[test]
    fn parses_standalone_flags() {
        let mut cfg = AppConfig::default();
        let args: Vec<String> = [
            "--auto-start",
            "--elevated-relaunch",
            "--show-console",
            "--window-pos",
            "450,280",
            "--drop",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let opts = parse(&args, &mut cfg).unwrap();
        assert!(opts.auto_start);
        assert!(opts.elevated_relaunch);
        assert!(opts.show_console);
        assert_eq!(opts.window_pos, Some([450.0, 280.0]));
        assert!(cfg.module_config(crate::engine::modules::ModuleKind::Drop).enabled);
    }

    #[test]
    fn rejects_bad_args() {
        let mut cfg = AppConfig::default();
        assert!(parse(&["--nonsense".to_string(), "x".to_string()], &mut cfg).is_err());
        assert!(parse(&["--lag-time".to_string()], &mut cfg).is_err());
        assert!(parse(&["--lag-time".to_string(), "99999".to_string()], &mut cfg).is_err());
    }
}
