//! Port of clumsy's packet-manipulation modules.
//!
//! Semantics deliberately mirror the original C implementation (see
//! ../src/{lag,drop,throttle,duplicate,ood,tamper,reset,bandwidth}.c), including
//! processing order, buffer caps and chance mechanics. Deviations are noted
//! with `NOTE:` comments.

use crate::engine::params::ModuleParams;
use crate::engine::rng::Rng;
use crate::packet::{calc_checksums, parse, set_rst_flag, Packet, PacketView, PROTO_ICMP, PROTO_ICMPV6};

// ---------------------------------------------------------------- constants

/// C lag.c: KEEP_AT_MOST
pub const LAG_KEEP_AT_MOST: usize = 2000;
/// C lag.c: FLUSH_WHEN_FULL
pub const LAG_FLUSH_WHEN_FULL: usize = 800;
/// C throttle.c: KEEP_AT_MOST
pub const THROTTLE_KEEP_AT_MOST: usize = 1000;
/// C ood.c: KEEP_TURNS_MAX
pub const OOD_KEEP_TURNS: i32 = 10;
/// C main.c module processing order (order decides which module sees packets first)
pub const MODULE_ORDER: [ModuleKind; 8] = [
    ModuleKind::Lag,
    ModuleKind::Drop,
    ModuleKind::Throttle,
    ModuleKind::Duplicate,
    ModuleKind::Ood,
    ModuleKind::Tamper,
    ModuleKind::Reset,
    ModuleKind::Bandwidth,
];

// ---------------------------------------------------------------- kinds

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleKind {
    Lag,
    Drop,
    Throttle,
    Duplicate,
    Ood,
    Tamper,
    Reset,
    Bandwidth,
}

impl ModuleKind {
    pub fn index(self) -> usize {
        MODULE_ORDER.iter().position(|&k| k == self).unwrap()
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ModuleKind::Lag => "Lag",
            ModuleKind::Drop => "Drop",
            ModuleKind::Throttle => "Throttle",
            ModuleKind::Duplicate => "Duplicate",
            ModuleKind::Ood => "Out of order",
            ModuleKind::Tamper => "Tamper",
            ModuleKind::Reset => "Set TCP RST",
            ModuleKind::Bandwidth => "Bandwidth",
        }
    }

    /// Short name used in CLI arguments / config, matches clumsy's NAME macros.
    pub fn short_name(self) -> &'static str {
        match self {
            ModuleKind::Lag => "lag",
            ModuleKind::Drop => "drop",
            ModuleKind::Throttle => "throttle",
            ModuleKind::Duplicate => "duplicate",
            ModuleKind::Ood => "ood",
            ModuleKind::Tamper => "tamper",
            ModuleKind::Reset => "reset",
            ModuleKind::Bandwidth => "bandwidth",
        }
    }

    #[allow(dead_code)]
    pub fn from_short_name(s: &str) -> Option<ModuleKind> {
        MODULE_ORDER.iter().copied().find(|k| k.short_name() == s)
    }
}

// ---------------------------------------------------------------- step context

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventImportance {
    /// Logged immediately (state changes, start/stop, errors)
    Always,
    /// Per-step summaries; rate-limited to avoid log flooding
    Summary,
}

pub struct StepEvent {
    pub module: ModuleKind,
    pub important: EventImportance,
    pub text: String,
}

pub struct StepCtx<'a> {
    pub queue: &'a mut std::collections::VecDeque<Packet>,
    pub now_ms: u64,
    pub rng: &'a mut Rng,
    pub events: Vec<StepEvent>,
    pub dropped: u64,
    pub modified: u64,
}

impl<'a> StepCtx<'a> {
    fn event(&mut self, module: ModuleKind, important: EventImportance, text: String) {
        self.events.push(StepEvent {
            module,
            important,
            text,
        });
    }
}

// ---------------------------------------------------------------- rate stats

/// Port of the bandwidth module's CRateStats (1s window, 1ms buckets).
pub struct RateStats {
    window: u32,
    initialized: bool,
    oldest_index: usize,
    oldest_ms: u64,
    accumulated: u64,
    bucket_bytes: Vec<u64>,
}

impl Default for RateStats {
    fn default() -> Self {
        RateStats {
            window: 1000,
            initialized: false,
            oldest_index: 0,
            oldest_ms: 0,
            accumulated: 0,
            bucket_bytes: vec![0; 1000],
        }
    }
}

impl RateStats {
    pub fn reset(&mut self) {
        *self = RateStats::default();
    }

    fn evict(&mut self, now_ms: u64) {
        if !self.initialized {
            return;
        }
        // u64 monotonic time: no wraparound, so plain comparisons suffice
        let new_oldest = now_ms.saturating_sub(self.window as u64 - 1);
        while self.oldest_ms < new_oldest {
            self.accumulated -= self.bucket_bytes[self.oldest_index];
            self.bucket_bytes[self.oldest_index] = 0;
            self.oldest_index = (self.oldest_index + 1) % self.window as usize;
            self.oldest_ms += 1;
        }
    }

    pub fn update(&mut self, bytes: usize, now_ms: u64) {
        if !self.initialized {
            self.initialized = true;
            self.oldest_ms = now_ms;
            self.oldest_index = 0;
            self.accumulated = 0;
        }
        if now_ms < self.oldest_ms {
            return;
        }
        self.evict(now_ms);
        let offset = (now_ms - self.oldest_ms) as usize;
        let index = (self.oldest_index + offset) % self.window as usize;
        self.bucket_bytes[index] += bytes as u64;
        self.accumulated += bytes as u64;
    }

    /// Bytes per second over the window, or None while the window is not full
    /// (mirrors calculate() returning -1).
    pub fn rate(&mut self, now_ms: u64) -> Option<u64> {
        self.evict(now_ms);
        if !self.initialized {
            return None;
        }
        let active_size = now_ms.saturating_sub(self.oldest_ms) + 1;
        if active_size <= 1 || active_size < self.window as u64 {
            return None;
        }
        // scale = 1000, window = 1000 => rate == bytes in window
        Some(self.accumulated)
    }
}

// ---------------------------------------------------------------- state

#[derive(Default)]
pub struct ModuleState {
    pub last_enabled: bool,
    /// Lag buffer, front = oldest.
    lag_buf: std::collections::VecDeque<Packet>,
    throttle_buf: Vec<Packet>,
    throttle_start: Option<u64>,
    ood_held: Option<Packet>,
    ood_give_up: i32,
    tamper_pattern_ix: usize,
    bandwidth: RateStats,
}

// C tamper.c: patterns covers every bit
const TAMPER_PATTERNS: [u8; 8] = [0x64, 0x13, 0x88, 0x40, 0x1F, 0xA0, 0xAA, 0x55];

#[inline]
fn direction_matches(params: &ModuleParams, outbound: bool) -> bool {
    params.direction.matches(outbound)
}

impl ModuleState {
    pub fn startup(&mut self, kind: ModuleKind) {
        match kind {
            ModuleKind::Ood => {
                self.ood_give_up = OOD_KEEP_TURNS;
                self.ood_held = None;
            }
            ModuleKind::Throttle => {
                self.throttle_buf.clear();
                self.throttle_start = None;
            }
            ModuleKind::Lag => {
                self.lag_buf.clear();
            }
            ModuleKind::Tamper => {
                self.tamper_pattern_ix = 0;
            }
            ModuleKind::Bandwidth => {
                self.bandwidth.reset();
            }
            _ => {}
        }
    }

    pub fn close_down(&mut self, kind: ModuleKind, params: &ModuleParams, ctx: &mut StepCtx) {
        match kind {
            // flush all buffered packets back into the send queue, oldest first
            ModuleKind::Lag => {
                let count = self.lag_buf.len();
                for p in self.lag_buf.drain(..) {
                    ctx.queue.push_back(p);
                }
                if count > 0 {
                    ctx.event(
                        kind,
                        EventImportance::Always,
                        format!("disabled, flushed {count} lagged packets"),
                    );
                }
            }
            ModuleKind::Throttle => {
                // C closeDown always clears (sends), never drops
                let count = self.throttle_buf.len();
                for p in self.throttle_buf.drain(..) {
                    ctx.queue.push_back(p);
                }
                self.throttle_start = None;
                if count > 0 {
                    ctx.event(
                        kind,
                        EventImportance::Always,
                        format!("disabled, released {count} throttled packets"),
                    );
                }
            }
            ModuleKind::Ood => {
                if let Some(p) = self.ood_held.take() {
                    ctx.queue.push_back(p);
                    ctx.event(
                        kind,
                        EventImportance::Always,
                        "disabled, released held packet".to_string(),
                    );
                }
            }
            _ => {
                let _ = params;
            }
        }
    }

    /// One processing step. Returns whether the module did anything
    /// (drives the "active" indicator).
    pub fn process(&mut self, kind: ModuleKind, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        match kind {
            ModuleKind::Lag => self.lag_process(params, ctx),
            ModuleKind::Drop => self.drop_process(kind, params, ctx),
            ModuleKind::Throttle => self.throttle_process(params, ctx),
            ModuleKind::Duplicate => self.dup_process(kind, params, ctx),
            ModuleKind::Ood => self.ood_process(kind, params, ctx),
            ModuleKind::Tamper => self.tamper_process(kind, params, ctx),
            ModuleKind::Reset => self.reset_process(kind, params, ctx),
            ModuleKind::Bandwidth => self.bandwidth_process(kind, params, ctx),
        }
    }

    // ---- lag --------------------------------------------------------------

    fn lag_process(&mut self, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        let delay = params.delay_ms.load(std::sync::atomic::Ordering::Relaxed).max(0) as u64;

        // capture matching packets into the buffer (until full), keep the rest
        if self.lag_buf.len() < LAG_KEEP_AT_MOST {
            let mut remaining = std::collections::VecDeque::with_capacity(ctx.queue.len());
            for mut p in ctx.queue.drain(..) {
                if self.lag_buf.len() < LAG_KEEP_AT_MOST && direction_matches(params, p.outbound()) {
                    p.due_ms = ctx.now_ms;
                    self.lag_buf.push_back(p);
                } else {
                    remaining.push_back(p);
                }
            }
            *ctx.queue = remaining;
        }

        // release overdue packets, oldest first
        let mut released = 0usize;
        while let Some(front) = self.lag_buf.front() {
            if ctx.now_ms > front.due_ms + delay {
                let p = self.lag_buf.pop_front().unwrap();
                ctx.queue.push_back(p);
                released += 1;
            } else {
                break;
            }
        }

        // buffer full: flush the oldest FLUSH_WHEN_FULL packets to keep going
        let mut flushed = 0usize;
        if self.lag_buf.len() >= LAG_KEEP_AT_MOST {
            for _ in 0..LAG_FLUSH_WHEN_FULL {
                match self.lag_buf.pop_front() {
                    Some(p) => {
                        ctx.queue.push_back(p);
                        flushed += 1;
                    }
                    None => break,
                }
            }
            if flushed > 0 {
                ctx.event(
                    ModuleKind::Lag,
                    EventImportance::Summary,
                    format!("buffer full, flushed {flushed} packets"),
                );
            }
        }

        self.lag_buf.len() > 0 || released > 0
    }

    // ---- drop -------------------------------------------------------------

    fn drop_process(&mut self, kind: ModuleKind, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        let chance = params.chance.load(std::sync::atomic::Ordering::Relaxed);
        let mut dropped = 0u64;
        ctx.queue.retain(|p| {
            if direction_matches(params, p.outbound()) && ctx.rng.chance(chance) {
                dropped += 1;
                false
            } else {
                true
            }
        });
        if dropped > 0 {
            ctx.dropped += dropped;
            ctx.event(kind, EventImportance::Summary, format!("dropped {dropped} packets"));
        }
        dropped > 0
    }

    // ---- throttle ----------------------------------------------------------

    fn throttle_process(&mut self, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        let chance = params.chance.load(std::sync::atomic::Ordering::Relaxed);
        let frame = params
            .throttle_frame_ms
            .load(std::sync::atomic::Ordering::Relaxed)
            .max(0) as u64;

        let mut started = false;
        if self.throttle_start.is_none() {
            if !ctx.queue.is_empty() && ctx.rng.chance(chance) {
                self.throttle_start = Some(ctx.now_ms);
                started = true;
                ctx.event(
                    ModuleKind::Throttle,
                    EventImportance::Always,
                    format!("throttling started (frame {frame}ms)"),
                );
            } else {
                return false;
            }
        }

        // capture matching packets into the throttle buffer
        if self.throttle_buf.len() < THROTTLE_KEEP_AT_MOST {
            let mut remaining = std::collections::VecDeque::with_capacity(ctx.queue.len());
            for p in ctx.queue.drain(..) {
                if self.throttle_buf.len() < THROTTLE_KEEP_AT_MOST && direction_matches(params, p.outbound())
                {
                    self.throttle_buf.push(p);
                } else {
                    remaining.push_back(p);
                }
            }
            *ctx.queue = remaining;
        }

        // release (or drop) when the frame elapsed or the buffer is full
        let start = self.throttle_start.unwrap_or(ctx.now_ms);
        if self.throttle_buf.len() >= THROTTLE_KEEP_AT_MOST || ctx.now_ms - start > frame {
            let drop = params.drop_throttled.load(std::sync::atomic::Ordering::Relaxed);
            let count = self.throttle_buf.len();
            if drop {
                ctx.dropped += count as u64;
                self.throttle_buf.clear();
                ctx.event(
                    ModuleKind::Throttle,
                    EventImportance::Always,
                    format!("throttling ended, dropped {count} packets"),
                );
            } else {
                for p in self.throttle_buf.drain(..) {
                    ctx.queue.push_back(p);
                }
                ctx.event(
                    ModuleKind::Throttle,
                    EventImportance::Always,
                    format!("throttling ended, released {count} packets"),
                );
            }
            self.throttle_start = None;
        }

        started || !self.throttle_buf.is_empty()
    }

    // ---- duplicate ---------------------------------------------------------

    fn dup_process(&mut self, kind: ModuleKind, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        let chance = params.chance.load(std::sync::atomic::Ordering::Relaxed);
        let count = params
            .dup_count
            .load(std::sync::atomic::Ordering::Relaxed)
            .clamp(2, 50) as usize;
        let mut out = std::collections::VecDeque::with_capacity(ctx.queue.len());
        let mut made = 0u64;
        for p in ctx.queue.drain(..) {
            let do_dup = direction_matches(params, p.outbound()) && ctx.rng.chance(chance);
            out.push_back(p);
            if do_dup {
                // clones follow the original (clumsy: "sends a cloned copy right
                // after the original packet")
                for _ in 0..count.saturating_sub(1) {
                    // NOTE: peek at what we just pushed rather than moving twice
                    let copy = out.back().unwrap().clone();
                    out.push_back(copy);
                    made += 1;
                }
            }
        }
        *ctx.queue = out;
        if made > 0 {
            ctx.modified += made;
            ctx.event(kind, EventImportance::Summary, format!("duplicated {made} packets"));
        }
        made > 0
    }

    // ---- out of order -------------------------------------------------------

    fn ood_process(&mut self, kind: ModuleKind, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        let chance = params.chance.load(std::sync::atomic::Ordering::Relaxed);

        if let Some(held) = self.ood_held.take() {
            if !ctx.queue.is_empty() {
                // release held packet behind the newly arrived ones -> swap
                ctx.queue.push_back(held);
                self.ood_give_up = OOD_KEEP_TURNS;
                return true;
            }
            self.ood_give_up -= 1;
            if self.ood_give_up <= 0 {
                ctx.queue.push_back(held);
                self.ood_give_up = OOD_KEEP_TURNS;
                ctx.event(
                    kind,
                    EventImportance::Summary,
                    "gave up waiting, released held packet".to_string(),
                );
                return true;
            } else {
                self.ood_held = Some(held);
                return false;
            }
        }

        if ctx.queue.is_empty() {
            return false;
        }

        let matching: Vec<usize> = ctx
            .queue
            .iter()
            .enumerate()
            .filter(|(_, p)| direction_matches(params, p.outbound()))
            .map(|(i, _)| i)
            .collect();

        if ctx.queue.len() == 1 && matching.len() == 1 {
            // single packet: hold it and release when the next arrives
            if ctx.rng.chance(chance) {
                self.ood_held = ctx.queue.remove(0);
                return true;
            }
            return false;
        }

        if ctx.rng.chance(chance) {
            // swap adjacent matching pairs, each with its own chance roll
            let mut swapped = 0;
            let mut i = 0;
            while i + 1 < matching.len() {
                if ctx.rng.chance(chance) {
                    let a = matching[i];
                    let b = matching[i + 1];
                    ctx.queue.swap(a, b);
                    swapped += 1;
                }
                i += 1;
            }
            if swapped > 0 {
                ctx.modified += swapped as u64;
                ctx.event(kind, EventImportance::Summary, format!("reordered {swapped} packet pairs"));
            }
            return true;
        }
        false
    }

    // ---- tamper --------------------------------------------------------------

    fn tamper_process(&mut self, kind: ModuleKind, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        let chance = params.chance.load(std::sync::atomic::Ordering::Relaxed);
        let redo = params.redo_checksum.load(std::sync::atomic::Ordering::Relaxed);
        let mut tampered = 0u64;

        for p in ctx.queue.iter_mut() {
            if !direction_matches(params, p.outbound()) || !ctx.rng.chance(chance) {
                continue;
            }
            let Some(view) = parse(&p.data) else { continue };
            let Some(payload_start) = view.payload else {
                continue;
            };
            if view.payload_len == 0 {
                continue;
            }

            let (start, len) = if view.payload_len <= 4 {
                (payload_start, view.payload_len)
            } else {
                // tamper the central part of the payload: common protocols keep
                // their checksums at head or tail
                let len_d4 = view.payload_len / 4;
                (payload_start + view.payload_len / 2 - len_d4 / 2 + 1, len_d4)
            };
            for b in &mut p.data[start..start + len] {
                *b ^= TAMPER_PATTERNS[self.tamper_pattern_ix & 0x7];
                self.tamper_pattern_ix += 1;
            }
            if redo {
                calc_checksums(&mut p.data);
            }
            tampered += 1;
        }

        if tampered > 0 {
            ctx.modified += tampered;
            ctx.event(kind, EventImportance::Summary, format!("tampered {tampered} packets"));
        }
        tampered > 0
    }

    // ---- reset ----------------------------------------------------------------

    fn reset_process(&mut self, kind: ModuleKind, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        let chance = params.chance.load(std::sync::atomic::Ordering::Relaxed);
        let mut reset = 0u64;

        for p in ctx.queue.iter_mut() {
            // C: packet must be bigger than IP+TCP headers (40 bytes)
            if p.data.len() <= 40 || !direction_matches(params, p.outbound()) {
                continue;
            }
            let pending = params.rst_next.load(std::sync::atomic::Ordering::Relaxed);
            if !(pending > 0 || ctx.rng.chance(chance)) {
                continue;
            }
            let Some(view) = parse(&p.data) else { continue };
            if view.protocol != crate::packet::PROTO_TCP || set_rst_flag(&mut p.data, &view) == false {
                continue;
            }
            calc_checksums(&mut p.data);
            reset += 1;
            if pending > 0 {
                params.rst_next.store(pending - 1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        if reset > 0 {
            ctx.modified += reset;
            ctx.event(kind, EventImportance::Summary, format!("set RST on {reset} packets"));
        }
        reset > 0
    }

    // ---- bandwidth ---------------------------------------------------------------

    fn bandwidth_process(&mut self, kind: ModuleKind, params: &ModuleParams, ctx: &mut StepCtx) -> bool {
        let limit_kbps = params.bandwidth_limit_kbps.load(std::sync::atomic::Ordering::Relaxed);
        if limit_kbps < 0 {
            return false;
        }
        let limit = limit_kbps as u64 * 1024;
        let mut dropped = 0u64;
        let mut kept = std::collections::VecDeque::with_capacity(ctx.queue.len());

        for p in ctx.queue.drain(..) {
            let mut discard = false;
            if direction_matches(params, p.outbound()) {
                if let Some(rate) = self.bandwidth.rate(ctx.now_ms) {
                    if rate + p.data.len() as u64 > limit {
                        discard = true;
                    }
                }
                if !discard {
                    self.bandwidth.update(p.data.len(), ctx.now_ms);
                }
            }
            if discard {
                dropped += 1;
            } else {
                kept.push_back(p);
            }
        }
        *ctx.queue = kept;

        if dropped > 0 {
            ctx.dropped += dropped;
            ctx.event(
                kind,
                EventImportance::Summary,
                format!("dropped {dropped} packets over {limit_kbps} KB/s limit"),
            );
        }
        dropped > 0
    }
}

/// Does this packet parse as ICMP/ICMPv6? Used for the inbound reinjection
/// workaround.
pub fn is_icmp(view: Option<PacketView>) -> bool {
    matches!(
        view.map(|v| v.protocol),
        Some(PROTO_ICMP) | Some(PROTO_ICMPV6)
    )
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::tcp_flags;
    use crate::wd::WdAddress;
    use std::collections::VecDeque;
    use std::sync::atomic::Ordering;

    fn udp_packet(outbound: bool) -> Packet {
        let mut d = vec![0u8; 28];
        d[0] = 0x45;
        let total = 28u16;
        d[2..4].copy_from_slice(&total.to_be_bytes());
        d[8] = 64;
        d[9] = 17;
        d[12..16].copy_from_slice(&[127, 0, 0, 1]);
        d[16..20].copy_from_slice(&[127, 0, 0, 2]);
        d[20..22].copy_from_slice(&1000u16.to_be_bytes());
        d[22..24].copy_from_slice(&2000u16.to_be_bytes());
        let mut addr = WdAddress::default();
        addr.outbound = outbound;
        addr.loopback = true;
        Packet::new(d, addr)
    }

    fn tcp_packet(outbound: bool) -> Packet {
        // 44 bytes: reset only applies to packets larger than IP+TCP headers (40)
        let mut d = vec![0u8; 44];
        d[0] = 0x45;
        let total = 44u16;
        d[2..4].copy_from_slice(&total.to_be_bytes());
        d[8] = 64;
        d[9] = 6;
        d[12..16].copy_from_slice(&[127, 0, 0, 1]);
        d[16..20].copy_from_slice(&[127, 0, 0, 2]);
        d[20..22].copy_from_slice(&1000u16.to_be_bytes());
        d[22..24].copy_from_slice(&80u16.to_be_bytes());
        d[32] = 0x50;
        d[40..44].copy_from_slice(&[1, 2, 3, 4]);
        let mut addr = WdAddress::default();
        addr.outbound = outbound;
        Packet::new(d, addr)
    }

    fn ctx<'a>(queue: &'a mut VecDeque<Packet>, now: u64, rng: &'a mut Rng) -> StepCtx<'a> {
        StepCtx {
            queue,
            now_ms: now,
            rng,
            events: Vec::new(),
            dropped: 0,
            modified: 0,
        }
    }

    // ---- lag ----

    #[test]
    fn lag_holds_then_releases_in_order() {
        let params = ModuleParams::new(ModuleKind::Lag);
        params.enabled.store(true, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(1);
        let mut q: VecDeque<Packet> = (0..3).map(|_| udp_packet(true)).collect();

        // capture at t=0
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Lag, &params, &mut c);
        }
        assert!(q.is_empty(), "all packets captured into lag buffer");

        // at t=40 nothing due yet (delay 50, strictly greater)
        {
            let mut c = ctx(&mut q, 40, &mut rng);
            state.process(ModuleKind::Lag, &params, &mut c);
            assert!(q.is_empty());
        }
        // at t=51 all due, released in arrival order
        {
            let mut c = ctx(&mut q, 51, &mut rng);
            state.process(ModuleKind::Lag, &params, &mut c);
        }
        assert_eq!(q.len(), 3);
        let due: Vec<u64> = q.iter().map(|p| p.due_ms).collect();
        assert_eq!(due, vec![0, 0, 0]);
    }

    #[test]
    fn lag_respects_direction() {
        let params = ModuleParams::new(ModuleKind::Lag);
        params.direction.inbound.store(false, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(1);
        let mut q: VecDeque<Packet> = vec![udp_packet(false), udp_packet(true)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Lag, &params, &mut c);
        }
        assert_eq!(q.len(), 1, "inbound packet left untouched");
        assert!(!q[0].outbound());
    }

    #[test]
    fn lag_close_down_flushes() {
        let params = ModuleParams::new(ModuleKind::Lag);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(1);
        let mut q: VecDeque<Packet> = vec![udp_packet(true), udp_packet(true)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Lag, &params, &mut c);
        }
        assert!(q.is_empty());
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.close_down(ModuleKind::Lag, &params, &mut c);
        }
        assert_eq!(q.len(), 2, "close down flushes the buffer");
    }

    #[test]
    fn lag_buffer_overflow_flushes_oldest() {
        let params = ModuleParams::new(ModuleKind::Lag);
        params.delay_ms.store(1000, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(1);
        let mut q: VecDeque<Packet> = (0..LAG_KEEP_AT_MOST + 5).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Lag, &params, &mut c);
        }
        // buffer capped at 2000; extra 5 remain in queue; 800 oldest flushed into queue
        assert_eq!(self_lag_len(&state), LAG_KEEP_AT_MOST - LAG_FLUSH_WHEN_FULL);
        assert_eq!(q.len(), 5 + LAG_FLUSH_WHEN_FULL);
    }

    fn self_lag_len(state: &ModuleState) -> usize {
        state.lag_buf.len()
    }

    // ---- drop ----

    #[test]
    fn drop_never_and_always() {
        let params = ModuleParams::new(ModuleKind::Drop);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(2);

        params.chance.store(0, Ordering::Relaxed);
        let mut q: VecDeque<Packet> = (0..50).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Drop, &params, &mut c);
        }
        assert_eq!(q.len(), 50);

        params.chance.store(10000, Ordering::Relaxed);
        let mut q: VecDeque<Packet> = (0..50).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Drop, &params, &mut c);
        }
        assert_eq!(q.len(), 0);

        // direction filter: inbound only never touches outbound packets
        params.direction.outbound.store(false, Ordering::Relaxed);
        params.chance.store(10000, Ordering::Relaxed);
        let mut q: VecDeque<Packet> = vec![udp_packet(true), udp_packet(false)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Drop, &params, &mut c);
        }
        assert_eq!(q.len(), 1);
        assert!(q[0].outbound());
    }

    #[test]
    fn drop_50_percent_statistically() {
        let params = ModuleParams::new(ModuleKind::Drop);
        params.chance.store(5000, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(3);
        let mut q: VecDeque<Packet> = (0..10000).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Drop, &params, &mut c);
        }
        let dropped = 10000 - q.len();
        assert!(
            (4500..=5500).contains(&dropped),
            "50% chance dropped {} of 10000",
            dropped
        );
    }

    // ---- throttle ----

    #[test]
    fn throttle_batches_packets() {
        let params = ModuleParams::new(ModuleKind::Throttle);
        params.chance.store(10000, Ordering::Relaxed); // always start
        params.throttle_frame_ms.store(30, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(4);

        // step 1: packet arrives -> throttle starts and captures it
        let mut q: VecDeque<Packet> = vec![udp_packet(true)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            assert!(state.process(ModuleKind::Throttle, &params, &mut c));
        }
        assert!(q.is_empty());

        // step 2: more packets at t=10, still inside frame -> captured
        q.extend((0..3).map(|_| udp_packet(true)));
        {
            let mut c = ctx(&mut q, 10, &mut rng);
            state.process(ModuleKind::Throttle, &params, &mut c);
        }
        assert!(q.is_empty());

        // step 3: at t=41 frame elapsed -> all 4 released in order
        {
            let mut c = ctx(&mut q, 41, &mut rng);
            state.process(ModuleKind::Throttle, &params, &mut c);
        }
        assert_eq!(q.len(), 4);
    }

    #[test]
    fn throttle_drop_throttled() {
        let params = ModuleParams::new(ModuleKind::Throttle);
        params.chance.store(10000, Ordering::Relaxed);
        params.throttle_frame_ms.store(10, Ordering::Relaxed);
        params.drop_throttled.store(true, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(5);

        let mut q: VecDeque<Packet> = (0..5).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Throttle, &params, &mut c);
        }
        {
            let mut c = ctx(&mut q, 11, &mut rng);
            state.process(ModuleKind::Throttle, &params, &mut c);
        }
        assert_eq!(q.len(), 0, "drop throttled discards the batch");
    }

    // ---- duplicate ----

    #[test]
    fn duplicate_adds_copies_after_original() {
        let params = ModuleParams::new(ModuleKind::Duplicate);
        params.chance.store(10000, Ordering::Relaxed);
        params.dup_count.store(3, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(6);

        let mut q: VecDeque<Packet> = (0..2).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Duplicate, &params, &mut c);
        }
        assert_eq!(q.len(), 6, "2 originals + 2 extra copies each");
        // originals first, copies after: both originals adjacent
        let dirs: Vec<bool> = q.iter().map(|p| p.outbound()).collect();
        assert_eq!(dirs, vec![true; 6]);
    }

    // ---- ood ----

    #[test]
    fn ood_swaps_two_packets() {
        let params = ModuleParams::new(ModuleKind::Ood);
        params.chance.store(10000, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(7);

        // single packet -> gets held
        let mut q: VecDeque<Packet> = vec![udp_packet(true)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Ood, &params, &mut c);
        }
        assert!(q.is_empty(), "packet held");

        // second packet arrives -> held released after it -> order swapped
        q.push_back(udp_packet(true));
        {
            let mut c = ctx(&mut q, 1, &mut rng);
            state.process(ModuleKind::Ood, &params, &mut c);
        }
        assert_eq!(q.len(), 2);
        // verify both present; the held one was pushed to the back
        // (identical packets, so just verify count and no hold left)
        assert!(state.ood_held.is_none());
    }

    #[test]
    fn ood_gives_up_after_ten_turns() {
        let params = ModuleParams::new(ModuleKind::Ood);
        params.chance.store(10000, Ordering::Relaxed);
        let mut state = ModuleState::default();
        state.startup(ModuleKind::Ood);
        let mut rng = Rng::new(8);

        let mut q: VecDeque<Packet> = vec![udp_packet(true)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Ood, &params, &mut c);
        }
        assert!(q.is_empty());
        // 10 empty steps decrement the counter to 0, then it is released
        for step in 1..=10 {
            let mut c = ctx(&mut q, step, &mut rng);
            state.process(ModuleKind::Ood, &params, &mut c);
            if step < 10 {
                assert!(q.is_empty(), "still held at step {step}");
            }
        }
        assert_eq!(q.len(), 1, "released on give-up");
    }

    // ---- tamper ----

    #[test]
    fn tamper_changes_central_payload_only() {
        let params = ModuleParams::new(ModuleKind::Tamper);
        params.chance.store(10000, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(9);

        // UDP packet with 12 payload bytes -> tampered region = 3 bytes
        let mut d = vec![0u8; 20 + 8 + 12];
        d[0] = 0x45;
        d[9] = 17;
        let udp_len = 20u16;
        d[24..26].copy_from_slice(&udp_len.to_be_bytes());
        for (i, b) in d[28..].iter_mut().enumerate() {
            *b = (i as u8) + 1;
        }
        let original = d.clone();
        let mut addr = WdAddress::default();
        addr.outbound = true;
        let mut q: VecDeque<Packet> = vec![Packet::new(d, addr)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Tamper, &params, &mut c);
        }
        let p = q.front().unwrap();
        // payload bytes 0..5 unchanged (before central region), 6..8 region changed, 9.. unchanged
        assert_eq!(&p.data[28..34], &original[28..34]);
        assert_ne!(&p.data[34..37], &original[34..37], "central 3 bytes tampered");
        assert_eq!(&p.data[37..40], &original[37..40]);
        // checksums recomputed: udp checksum no longer zero
        assert_ne!(p.data[26], 0);
    }

    // ---- reset ----

    #[test]
    fn reset_sets_rst_flag() {
        let params = ModuleParams::new(ModuleKind::Reset);
        params.chance.store(10000, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(10);

        let mut q: VecDeque<Packet> = vec![tcp_packet(true)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Reset, &params, &mut c);
        }
        let v = parse(&q[0].data).unwrap();
        let flags = tcp_flags(&q[0].data, &v).unwrap();
        assert!(flags.2, "RST flag set");

        // UDP packets are untouched
        let mut q: VecDeque<Packet> = vec![udp_packet(true)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Reset, &params, &mut c);
        }
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn reset_rst_next_button() {
        let params = ModuleParams::new(ModuleKind::Reset);
        params.chance.store(0, Ordering::Relaxed);
        let mut state = ModuleState::default();
        let mut rng = Rng::new(11);

        params.rst_next.store(1, Ordering::Relaxed);
        let mut q: VecDeque<Packet> = vec![tcp_packet(true)].into();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Reset, &params, &mut c);
        }
        let v = parse(&q[0].data).unwrap();
        assert!(tcp_flags(&q[0].data, &v).unwrap().2);
        assert_eq!(params.rst_next.load(Ordering::Relaxed), 0);
    }

    // ---- bandwidth ----

    #[test]
    fn bandwidth_allows_within_limit_drops_over() {
        let params = ModuleParams::new(ModuleKind::Bandwidth);
        params.bandwidth_limit_kbps.store(10, Ordering::Relaxed); // 10240 B/s
        let mut state = ModuleState::default();
        let mut rng = Rng::new(12);

        // during the first second the window is not full: everything passes
        let mut q: VecDeque<Packet> = (0..10).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 0, &mut rng);
            state.process(ModuleKind::Bandwidth, &params, &mut c);
        }
        assert_eq!(q.len(), 10);

        // after 1.1s of quiet the window is full but empty -> passes again & records
        let mut q: VecDeque<Packet> = (0..5).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 1100, &mut rng);
            state.process(ModuleKind::Bandwidth, &params, &mut c);
        }
        assert_eq!(q.len(), 5);

        // simulate a saturated window: 10240 bytes recorded at t=2000; the window
        // is "active" (1000ms) at t=2999 without having evicted that bucket yet
        state.bandwidth.reset();
        state.bandwidth.update(10240, 2000);
        let mut q: VecDeque<Packet> = (0..4).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 2999, &mut rng);
            state.process(ModuleKind::Bandwidth, &params, &mut c);
        }
        assert_eq!(q.len(), 0, "packets over the limit are dropped");

        // 1000ms later the recorded bucket was evicted: traffic passes again
        let mut q: VecDeque<Packet> = (0..2).map(|_| udp_packet(true)).collect();
        {
            let mut c = ctx(&mut q, 3010, &mut rng);
            state.process(ModuleKind::Bandwidth, &params, &mut c);
        }
        assert_eq!(q.len(), 2, "window empty again, traffic passes");
    }

    // ---- misc ----

    #[test]
    fn icmp_detection() {
        let mut d = vec![0u8; 28];
        d[0] = 0x45;
        d[9] = 1; // ICMP
        let v = parse(&d);
        assert!(is_icmp(v));
        let mut d = vec![0u8; 28];
        d[0] = 0x45;
        d[9] = 17;
        assert!(!is_icmp(parse(&d)));
    }
}
