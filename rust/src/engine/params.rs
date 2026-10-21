//! Live-tunable module parameters shared between the UI thread and the engine
//! threads via atomics (the Rust equivalent of clumsy's volatile shorts).

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU16};

/// Chance values are stored as u16 in [0, 10000], i.e. percent * 100,
/// exactly like clumsy. 10000 = always, 0 = never.
pub const CHANCE_MAX: u16 = 10000;

#[derive(Debug, Default)]
pub struct DirectionFlags {
    pub inbound: AtomicBool,
    pub outbound: AtomicBool,
}

impl DirectionFlags {
    pub fn both() -> DirectionFlags {
        DirectionFlags {
            inbound: AtomicBool::new(true),
            outbound: AtomicBool::new(true),
        }
    }

    #[inline]
    pub fn matches(&self, outbound_packet: bool) -> bool {
        if outbound_packet {
            self.outbound.load(std::sync::atomic::Ordering::Relaxed)
        } else {
            self.inbound.load(std::sync::atomic::Ordering::Relaxed)
        }
    }
}

#[derive(Debug)]
pub struct ModuleParams {
    pub enabled: AtomicBool,
    pub direction: DirectionFlags,
    /// [0, 10000]
    pub chance: AtomicU16,
    /// Lag: delay in ms [0, 15000]
    pub delay_ms: AtomicI32,
    /// Throttle: time frame in ms [0, 1000]
    pub throttle_frame_ms: AtomicI32,
    /// Throttle: drop instead of releasing the batch
    pub drop_throttled: AtomicBool,
    /// Duplicate: number of total copies [2, 50]
    pub dup_count: AtomicI32,
    /// Tamper: recompute checksums after tampering
    pub redo_checksum: AtomicBool,
    /// Reset: pending "RST next packet" presses
    pub rst_next: AtomicI32,
    /// Bandwidth: cap in KB/s [0, 99999]
    pub bandwidth_limit_kbps: AtomicI32,
}

impl ModuleParams {
    pub fn new(kind: super::ModuleKind) -> ModuleParams {
        use super::ModuleKind::*;
        let chance = match kind {
            Reset => 0, // off by default, RST is dangerous
            _ => 1000,  // 10%
        };
        ModuleParams {
            enabled: AtomicBool::new(false),
            direction: DirectionFlags::both(),
            chance: AtomicU16::new(chance),
            delay_ms: AtomicI32::new(50),
            throttle_frame_ms: AtomicI32::new(30),
            drop_throttled: AtomicBool::new(false),
            dup_count: AtomicI32::new(2),
            redo_checksum: AtomicBool::new(true),
            rst_next: AtomicI32::new(0),
            bandwidth_limit_kbps: AtomicI32::new(10),
        }
    }

    pub fn chance_percent(&self) -> f32 {
        self.chance.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0
    }

    pub fn set_chance_percent(&self, percent: f32) {
        let v = (percent * 100.0).round().clamp(0.0, CHANCE_MAX as f32) as u16;
        self.chance.store(v, std::sync::atomic::Ordering::Relaxed);
    }
}
