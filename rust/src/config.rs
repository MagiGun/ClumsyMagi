//! Persistent application configuration, stored as JSON under
//! %APPDATA%/ClumsyMagi/config.json. Extends clumsy's config.txt (which only
//! held filter presets) with UI state and all module parameters.

use crate::engine::modules::ModuleKind;
use crate::engine::params::ModuleParams;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub filter: String,
    pub builtin: bool,
}

/// Built-in presets, from clumsy's etc/config.txt plus the classic ones.
pub fn builtin_presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "all sending packets (outbound)".into(),
            filter: "outbound".into(),
            builtin: true,
        },
        Preset {
            name: "all receiving packets (inbound)".into(),
            filter: "inbound".into(),
            builtin: true,
        },
        Preset {
            name: "all traffic (inbound or outbound)".into(),
            filter: "inbound or outbound".into(),
            builtin: true,
        },
        Preset {
            name: "localhost ipv4 all".into(),
            filter: "outbound and loopback".into(),
            builtin: true,
        },
        Preset {
            name: "localhost ipv4 tcp".into(),
            filter: "tcp and outbound and loopback".into(),
            builtin: true,
        },
        Preset {
            name: "localhost ipv4 udp".into(),
            filter: "udp and outbound and loopback".into(),
            builtin: true,
        },
        Preset {
            name: "all ipv4 against specific ip".into(),
            filter: "ip.DstAddr == 198.51.100.1 or ip.SrcAddr == 198.51.100.1".into(),
            builtin: true,
        },
        Preset {
            name: "tcp ipv4 against specific ip".into(),
            filter: "tcp and (ip.DstAddr == 198.51.100.1 or ip.SrcAddr == 198.51.100.1)".into(),
            builtin: true,
        },
        Preset {
            name: "udp ipv4 against specific ip".into(),
            filter: "udp and (ip.DstAddr == 198.51.100.1 or ip.SrcAddr == 198.51.100.1)".into(),
            builtin: true,
        },
        Preset {
            name: "all ipv4 against port".into(),
            filter: "tcp.DstPort == 12354 or tcp.SrcPort == 12354 or udp.DstPort == 12354 or udp.SrcPort == 12354".into(),
            builtin: true,
        },
        Preset {
            name: "tcp ipv4 against port".into(),
            filter: "tcp and (tcp.DstPort == 12354 or tcp.SrcPort == 12354)".into(),
            builtin: true,
        },
        Preset {
            name: "udp ipv4 against port".into(),
            filter: "udp and (udp.DstPort == 12354 or udp.SrcPort == 12354)".into(),
            builtin: true,
        },
        Preset {
            name: "ipv6 all".into(),
            filter: "ipv6".into(),
            builtin: true,
        },
        Preset {
            name: "icmp ping outbound".into(),
            filter: "icmp and outbound".into(),
            builtin: true,
        },
    ]
}

pub fn load_presets_from_config_txt() -> Vec<Preset> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("config.txt"));
            candidates.push(dir.join("etc").join("config.txt"));
        }
    }
    candidates.push(PathBuf::from("config.txt"));
    candidates.push(PathBuf::from("etc").join("config.txt"));
    candidates.push(PathBuf::from("../etc/config.txt"));

    for path in candidates {
        if let Ok(content) = std::fs::read_to_string(&path) {
            let mut list = Vec::new();
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some((name, filter)) = trimmed.split_once(':') {
                    let name = name.trim().to_string();
                    let filter = filter.trim().to_string();
                    if !name.is_empty() && !filter.is_empty() {
                        list.push(Preset {
                            name,
                            filter,
                            builtin: true,
                        });
                    }
                }
            }
            if !list.is_empty() {
                return list;
            }
        }
    }
    Vec::new()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleConfig {
    pub enabled: bool,
    pub inbound: bool,
    pub outbound: bool,
    pub chance_percent: f32,
    pub delay_ms: Option<i32>,
    pub throttle_frame_ms: Option<i32>,
    pub drop_throttled: Option<bool>,
    pub dup_count: Option<i32>,
    pub redo_checksum: Option<bool>,
    pub bandwidth_limit_kbps: Option<i32>,
}

impl Default for ModuleConfig {
    /// Defaults mirror the original clumsy UI: both directions on, 10% chance
    /// (0% for the RST module), lag 50ms, throttle 30ms, duplicate x2,
    /// checksum redo on, bandwidth 10 KB/s.
    fn default() -> Self {
        ModuleConfig {
            enabled: false,
            inbound: true,
            outbound: true,
            chance_percent: 10.0,
            delay_ms: Some(50),
            throttle_frame_ms: Some(30),
            drop_throttled: Some(false),
            dup_count: Some(2),
            redo_checksum: Some(true),
            bandwidth_limit_kbps: Some(10),
        }
    }
}

impl ModuleConfig {
    pub fn for_kind(kind: ModuleKind) -> ModuleConfig {
        let mut cfg = ModuleConfig::default();
        // RST is dangerous: chance off by default, driven by the button
        if kind == ModuleKind::Reset {
            cfg.chance_percent = 0.0;
        }
        cfg
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: Theme,
    pub filter: String,
    pub presets: Vec<Preset>,
    pub modules: std::collections::BTreeMap<String, ModuleConfig>,
    /// start filtering immediately on launch (also set by --auto-start)
    pub auto_start: bool,
    pub show_log: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            theme: Theme::Dark,
            filter: "outbound and loopback".to_string(),
            presets: builtin_presets(),
            modules: Default::default(),
            auto_start: false,
            show_log: true,
        }
    }
}

impl AppConfig {
    pub fn module_config(&self, kind: ModuleKind) -> ModuleConfig {
        self.modules
            .get(kind.short_name())
            .cloned()
            .unwrap_or_else(|| ModuleConfig::for_kind(kind))
    }

    pub fn module_config_mut(&mut self, kind: ModuleKind) -> &mut ModuleConfig {
        self.modules
            .entry(kind.short_name().to_string())
            .or_insert_with(|| ModuleConfig::for_kind(kind))
    }

    /// Build fresh engine params from stored config.
    pub fn build_params(&self) -> Vec<Arc<ModuleParams>> {
        crate::engine::all_modules()
            .iter()
            .map(|kind| {
                let params = ModuleParams::new(*kind);
                let cfg = self.module_config(*kind);
                params.enabled.store(cfg.enabled, Ordering::Relaxed);
                params.direction.inbound.store(cfg.inbound, Ordering::Relaxed);
                params
                    .direction
                    .outbound
                    .store(cfg.outbound, Ordering::Relaxed);
                params.set_chance_percent(cfg.chance_percent);
                if let Some(v) = cfg.delay_ms {
                    params.delay_ms.store(v.clamp(0, 15000), Ordering::Relaxed);
                }
                if let Some(v) = cfg.throttle_frame_ms {
                    params.throttle_frame_ms.store(v.clamp(0, 1000), Ordering::Relaxed);
                }
                if let Some(v) = cfg.drop_throttled {
                    params.drop_throttled.store(v, Ordering::Relaxed);
                }
                if let Some(v) = cfg.dup_count {
                    params.dup_count.store(v.clamp(2, 50), Ordering::Relaxed);
                }
                if let Some(v) = cfg.redo_checksum {
                    params.redo_checksum.store(v, Ordering::Relaxed);
                }
                if let Some(v) = cfg.bandwidth_limit_kbps {
                    params.bandwidth_limit_kbps.store(v.clamp(0, 99999), Ordering::Relaxed);
                }
                Arc::new(params)
            })
            .collect()
    }

    /// Collect current live params back into the config.
    pub fn sync_from_params(&mut self, params: &[Arc<ModuleParams>]) {
        for kind in crate::engine::all_modules() {
            let p = &params[kind.index()];
            let cfg = ModuleConfig {
                enabled: p.enabled.load(Ordering::Relaxed),
                inbound: p.direction.inbound.load(Ordering::Relaxed),
                outbound: p.direction.outbound.load(Ordering::Relaxed),
                chance_percent: p.chance_percent(),
                delay_ms: Some(p.delay_ms.load(Ordering::Relaxed)),
                throttle_frame_ms: Some(p.throttle_frame_ms.load(Ordering::Relaxed)),
                drop_throttled: Some(p.drop_throttled.load(Ordering::Relaxed)),
                dup_count: Some(p.dup_count.load(Ordering::Relaxed)),
                redo_checksum: Some(p.redo_checksum.load(Ordering::Relaxed)),
                bandwidth_limit_kbps: Some(p.bandwidth_limit_kbps.load(Ordering::Relaxed)),
            };
            self.modules.insert(kind.short_name().to_string(), cfg);
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ClumsyMagi").join("config.json"))
}

pub fn load() -> AppConfig {
    let mut cfg = config_path()
        .and_then(|path| std::fs::read_to_string(&path).ok())
        .and_then(|text| serde_json::from_str::<AppConfig>(&text).ok())
        .unwrap_or_default();

    // always include any missing built-in presets
    for bp in builtin_presets() {
        if !cfg.presets.iter().any(|p| p.name == bp.name) {
            cfg.presets.push(bp);
        }
    }
    // merge presets loaded from config.txt
    for cp in load_presets_from_config_txt() {
        if !cfg.presets.iter().any(|p| p.name == cp.name) {
            cfg.presets.push(cp);
        }
    }
    cfg
}

pub fn save(cfg: &AppConfig) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("no config directory available".to_string());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create_dir_all: {e}"))?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| format!("serialize: {e}"))?;
    // write-then-rename for atomicity
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}
