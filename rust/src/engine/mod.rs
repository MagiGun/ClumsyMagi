//! The packet processing engine: recv thread + clock thread around a shared
//! packet queue, mirroring clumsy's divert.c architecture.
//!
//! - recv thread: blocking WinDivertRecv, then immediately runs one consume
//!   step and sends everything left in the queue (keeps pass-through latency low)
//! - clock thread: every 10ms, opportunistically runs a consume step so
//!   lagged/throttled packets get released even without new traffic, and
//!   performs the final flush+close on stop

pub mod modules;
pub mod params;
pub mod rng;

use crate::packet::{parse, swap_ip_addrs, Packet};
use crate::wd::{WdAddress, WinDivertLib, WINDIVERT_PARAM_QUEUE_LENGTH, WINDIVERT_PARAM_QUEUE_TIME};
use modules::{is_icmp, EventImportance, ModuleKind, ModuleState, StepCtx, MODULE_ORDER};
use params::ModuleParams;
use rng::Rng;
use std::collections::VecDeque;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const QUEUE_LEN: u64 = 2 << 10; // 2048, like clumsy
pub const QUEUE_TIME: u64 = 2 << 9; // 512ms, like clumsy
pub const CLOCK_TICK_MS: u64 = 10;

// ---------------------------------------------------------------- stats

#[derive(Debug, Default)]
pub struct Stats {
    pub captured: AtomicU64,
    pub sent: AtomicU64,
    pub dropped: AtomicU64,
    pub modified: AtomicU64,
    pub send_failed: AtomicU64,
}

impl Stats {
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            captured: self.captured.load(Ordering::Relaxed),
            sent: self.sent.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            modified: self.modified.load(Ordering::Relaxed),
            send_failed: self.send_failed.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StatsSnapshot {
    pub captured: u64,
    pub sent: u64,
    pub dropped: u64,
    pub modified: u64,
    pub send_failed: u64,
}

// ---------------------------------------------------------------- log

#[derive(Debug, Clone)]
pub struct LogEvent {
    pub unix_ms: u64,
    pub module: Option<ModuleKind>,
    pub text: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Traffic,
    Warn,
    #[allow(dead_code)]
    Error,
}

/// Bounded event log shared with the UI.
pub struct LogBuffer {
    events: Mutex<VecDeque<LogEvent>>,
    capacity: usize,
    /// per-module last unix_ms of a Summary event (rate limiting)
    last_summary_ms: Mutex<Vec<u64>>,
}

impl LogBuffer {
    fn new(capacity: usize) -> LogBuffer {
        LogBuffer {
            events: Mutex::new(VecDeque::with_capacity(256)),
            capacity,
            last_summary_ms: Mutex::new(vec![0; MODULE_ORDER.len()]),
        }
    }

    fn push(&self, ev: LogEvent) {
        let mut q = self.events.lock().unwrap_or_else(|e| e.into_inner());
        while q.len() >= self.capacity {
            q.pop_front();
        }
        q.push_back(ev);
    }

    fn push_summary(&self, ev: LogEvent, module: ModuleKind) {
        // rate-limit per-module summaries to 1 per 300ms
        let now = ev.unix_ms;
        let mut last = self.last_summary_ms.lock().unwrap_or_else(|e| e.into_inner());
        let slot = &mut last[module.index()];
        if now.saturating_sub(*slot) < 300 {
            return;
        }
        *slot = now;
        drop(last);
        self.push(ev);
    }

    pub fn snapshot(&self) -> Vec<LogEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------- shared state

struct EngineQueue {
    queue: VecDeque<Packet>,
    states: Vec<ModuleState>,
    rng: Rng,
    start: Instant,
}

impl EngineQueue {
    fn new(params_count: usize) -> EngineQueue {
        EngineQueue {
            queue: VecDeque::new(),
            states: (0..params_count).map(|_| ModuleState::default()).collect(),
            rng: Rng::from_entropy(),
            start: Instant::now(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

pub struct EngineShared {
    pub lib: Arc<WinDivertLib>,
    pub params: Vec<Arc<ModuleParams>>,
    pub stats: Stats,
    pub log: LogBuffer,
    /// wall-clock ms of the last module trigger (for the activity dots)
    pub last_triggered_ms: Vec<AtomicU64>,
    /// wall-clock ms of the last successful / failed send (for the global dot)
    pub send_ok_ms: AtomicU64,
    pub send_fail_ms: AtomicU64,
    pub running: AtomicBool,
    stopping: AtomicBool,
    handle: AtomicPtr<std::ffi::c_void>,
    queue: Mutex<EngineQueue>,
}

impl EngineShared {
    /// How recently (ms) a module was triggered; None = never this run.
    pub fn since_triggered(&self, kind: ModuleKind) -> Option<u64> {
        let t = self.last_triggered_ms[kind.index()].load(Ordering::Relaxed);
        if t == 0 {
            None
        } else {
            Some(unix_ms().saturating_sub(t))
        }
    }
}

// ---------------------------------------------------------------- step + send

impl EngineShared {
    /// Run all enabled modules over the queue, then is followed by send_all.
    fn consume_step(&self, eq: &mut EngineQueue) {
        let now = eq.now_ms();
        let (events, dropped, modified) = {
            let mut ctx = StepCtx {
                queue: &mut eq.queue,
                now_ms: now,
                rng: &mut eq.rng,
                events: Vec::new(),
                dropped: 0,
                modified: 0,
            };
            for (ix, kind) in MODULE_ORDER.iter().enumerate() {
                let params = &self.params[ix];
                let enabled = params.enabled.load(Ordering::Relaxed);
                let state = &mut eq.states[ix];
                if enabled {
                    if !state.last_enabled {
                        state.startup(*kind);
                        state.last_enabled = true;
                        self.log.push(LogEvent {
                            unix_ms: unix_ms(),
                            module: Some(*kind),
                            text: format!("{} enabled", kind.display_name()),
                            level: LogLevel::Info,
                        });
                    }
                    if state.process(*kind, params, &mut ctx) {
                        self.last_triggered_ms[ix].store(unix_ms(), Ordering::Relaxed);
                    }
                } else if state.last_enabled {
                    state.close_down(*kind, params, &mut ctx);
                    state.last_enabled = false;
                    self.log.push(LogEvent {
                        unix_ms: unix_ms(),
                        module: Some(*kind),
                        text: format!("{} disabled", kind.display_name()),
                        level: LogLevel::Info,
                    });
                }
            }
            (ctx.events, ctx.dropped, ctx.modified)
        };

        if dropped > 0 {
            self.stats.dropped.fetch_add(dropped, Ordering::Relaxed);
        }
        if modified > 0 {
            self.stats.modified.fetch_add(modified, Ordering::Relaxed);
        }

        // drain events: count traffic, log important ones, rate-limit summaries
        for ev in events {
            match ev.important {
                EventImportance::Always => {
                    self.log.push(LogEvent {
                        unix_ms: unix_ms(),
                        module: Some(ev.module),
                        text: ev.text,
                        level: LogLevel::Traffic,
                    });
                }
                EventImportance::Summary => {
                    self.log.push_summary(
                        LogEvent {
                            unix_ms: unix_ms(),
                            module: Some(ev.module),
                            text: ev.text,
                            level: LogLevel::Traffic,
                        },
                        ev.module,
                    );
                }
            }
        }
    }

    /// Send every packet currently queued.
    fn send_all(&self, handle: *mut std::ffi::c_void, eq: &mut EngineQueue) {
        while let Some(mut p) = eq.queue.pop_front() {
            match self.lib.send_on(handle, &mut p.data, &p.addr) {
                Ok(n) if n == p.data.len() => {
                    self.stats.sent.fetch_add(1, Ordering::Relaxed);
                    self.send_ok_ms.store(unix_ms(), Ordering::Relaxed);
                }
                Ok(_) => {
                    self.stats.send_failed.fetch_add(1, Ordering::Relaxed);
                    self.send_fail_ms.store(unix_ms(), Ordering::Relaxed);
                }
                Err(e) => {
                    let code = match &e {
                        crate::wd::WdError::Io(c) => *c,
                        _ => 0,
                    };
                    // WinDivert sometimes fails to reinject inbound ICMP packets;
                    // workaround (like clumsy): resend as outbound with swapped
                    // src/dst addresses.
                    let view = parse(&p.data);
                    if is_icmp(view) && !p.addr.outbound {
                        p.addr.outbound = true;
                        swap_ip_addrs(&mut p.data);
                        match self.lib.send_on(handle, &mut p.data, &p.addr) {
                            Ok(_) => {
                                self.stats.sent.fetch_add(1, Ordering::Relaxed);
                                self.send_ok_ms.store(unix_ms(), Ordering::Relaxed);
                                self.log.push(LogEvent {
                                    unix_ms: unix_ms(),
                                    module: None,
                                    text: "resent failed inbound ICMP packet as outbound".to_string(),
                                    level: LogLevel::Info,
                                });
                            }
                            Err(_) => {
                                self.stats.send_failed.fetch_add(1, Ordering::Relaxed);
                                self.send_fail_ms.store(unix_ms(), Ordering::Relaxed);
                                self.log_fail(code);
                            }
                        }
                    } else {
                        self.stats.send_failed.fetch_add(1, Ordering::Relaxed);
                        self.send_fail_ms.store(unix_ms(), Ordering::Relaxed);
                        self.log_fail(code);
                    }
                }
            }
        }
    }

    fn log_fail(&self, code: u32) {
        self.log.push(LogEvent {
            unix_ms: unix_ms(),
            module: None,
            text: format!("failed to reinject packet (Win32 error {code})"),
            level: LogLevel::Warn,
        });
    }
}

// ---------------------------------------------------------------- handle / API

pub struct EngineHandle {
    pub shared: Arc<EngineShared>,
    recv_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    clock_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl EngineHandle {
    /// Start filtering. Validates the filter first; returns friendly errors.
    pub fn start(
        lib: Arc<WinDivertLib>,
        filter: &str,
        params: Vec<Arc<ModuleParams>>,
    ) -> Result<EngineHandle, String> {
        if let Err(e) = lib.compile_filter(filter) {
            return Err(e);
        }
        let handle = match lib.open(filter) {
            Ok(h) => h,
            Err(e) => return Err(e.friendly()),
        };
        lib.set_param_on(handle, WINDIVERT_PARAM_QUEUE_LENGTH, QUEUE_LEN);
        lib.set_param_on(handle, WINDIVERT_PARAM_QUEUE_TIME, QUEUE_TIME);
        crate::platform::start_timer_resolution();

        let module_count = params.len();
        let shared = Arc::new(EngineShared {
            lib,
            params,
            stats: Stats::default(),
            log: LogBuffer::new(2000),
            last_triggered_ms: (0..module_count).map(|_| AtomicU64::new(0)).collect(),
            send_ok_ms: AtomicU64::new(0),
            send_fail_ms: AtomicU64::new(0),
            running: AtomicBool::new(true),
            stopping: AtomicBool::new(false),
            handle: AtomicPtr::new(handle),
            queue: Mutex::new(EngineQueue::new(module_count)),
        });

        shared.log.push(LogEvent {
            unix_ms: unix_ms(),
            module: None,
            text: format!("Started filtering with filter: {filter}"),
            level: LogLevel::Info,
        });

        let recv_shared = shared.clone();
        let clock_shared = shared.clone();
        let recv = std::thread::Builder::new()
            .name("clumsymagi-recv".into())
            .spawn(move || recv_loop(recv_shared))
            .map_err(|e| format!("failed to spawn recv thread: {e}"))?;
        let clock = std::thread::Builder::new()
            .name("clumsymagi-clock".into())
            .spawn(move || clock_loop(clock_shared))
            .map_err(|e| format!("failed to spawn clock thread: {e}"))?;

        Ok(EngineHandle {
            shared,
            recv_thread: Mutex::new(Some(recv)),
            clock_thread: Mutex::new(Some(clock)),
        })
    }

    /// Stop filtering: flush module buffers, send everything, close the handle.
    /// Blocks until both threads have exited (bounded wait).
    pub fn stop(&self) {
        let sh = &self.shared;
        if !sh.running.swap(false, Ordering::SeqCst) {
            return;
        }
        sh.stopping.store(true, Ordering::SeqCst);
        let handle = sh.handle.load(Ordering::SeqCst);
        if !handle.is_null() {
            // unblock the recv thread
            sh.lib.shutdown(handle);
        }
        if let Some(t) = self.recv_thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = t.join();
        }
        if let Some(t) = self.clock_thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = t.join();
        }
        crate::platform::end_timer_resolution();
        sh.log.push(LogEvent {
            unix_ms: unix_ms(),
            module: None,
            text: "Stopped filtering".to_string(),
            level: LogLevel::Info,
        });
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------- threads

fn recv_loop(shared: Arc<EngineShared>) {
    let mut buf = vec![0u8; crate::wd::MAX_PACKETSIZE];
    let mut addr_raw = [0u8; crate::wd::WINDIVERT_ADDRESS_SIZE];
    loop {
        let handle = shared.handle.load(Ordering::SeqCst);
        if handle.is_null() {
            break;
        }
        match shared.lib.recv_on(handle, &mut buf, &mut addr_raw) {
            Ok(len) => {
                let mut eq = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                if shared.stopping.load(Ordering::SeqCst) {
                    // user stopped while we were blocked: drop the last packet
                    // (same as clumsy: "Lost last recved packet but user stopped")
                    break;
                }
                let addr = WdAddress::from_raw(&addr_raw);
                eq.queue.push_back(Packet::new(buf[..len].to_vec(), addr));
                shared.stats.captured.fetch_add(1, Ordering::Relaxed);
                shared.consume_step(&mut eq);
                shared.send_all(handle, &mut eq);
                drop(eq);
            }
            Err(crate::wd::WdError::Io(232)) => {
                // ERROR_NO_DATA: handle was shut down
                break;
            }
            Err(crate::wd::WdError::Io(6)) => {
                // ERROR_INVALID_HANDLE: handle was closed
                break;
            }
            Err(crate::wd::WdError::Io(code)) => {
                shared.log.push(LogEvent {
                    unix_ms: unix_ms(),
                    module: None,
                    text: format!("recv failed (Win32 error {code})"),
                    level: LogLevel::Warn,
                });
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
}

fn clock_loop(shared: Arc<EngineShared>) {
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            let handle = shared.handle.swap(ptr::null_mut(), Ordering::SeqCst);
            if !handle.is_null() {
                let mut guard = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                let eq: &mut EngineQueue = &mut guard;
                // final close-down: flush all module buffers and send everything
                let now = eq.now_ms();
                for (ix, kind) in MODULE_ORDER.iter().enumerate() {
                    let params = &shared.params[ix];
                    if params.enabled.load(Ordering::Relaxed) && eq.states[ix].last_enabled {
                        let mut ctx = StepCtx {
                            queue: &mut eq.queue,
                            now_ms: now,
                            rng: &mut eq.rng,
                            events: Vec::new(),
                            dropped: 0,
                            modified: 0,
                        };
                        eq.states[ix].close_down(*kind, params, &mut ctx);
                        for ev in ctx.events {
                            shared.log.push(LogEvent {
                                unix_ms: unix_ms(),
                                module: Some(*kind),
                                text: ev.text,
                                level: LogLevel::Info,
                            });
                        }
                    }
                }
                shared.send_all(handle, eq);
                drop(guard);
                shared.lib.close_on(handle);
            }
            break;
        }
        let handle = shared.handle.load(Ordering::SeqCst);
        if !handle.is_null() {
            if let Ok(mut eq) = shared.queue.try_lock() {
                if !shared.stopping.load(Ordering::SeqCst) {
                    shared.consume_step(&mut eq);
                    shared.send_all(handle, &mut eq);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(CLOCK_TICK_MS));
    }
}

// ---------------------------------------------------------------- misc

/// Monotonic-ish module metadata for the UI.
pub fn all_modules() -> &'static [ModuleKind] {
    &MODULE_ORDER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_buffer_capacity() {
        let lb = LogBuffer::new(4);
        for i in 0..10 {
            lb.push(LogEvent {
                unix_ms: i,
                module: None,
                text: format!("e{i}"),
                level: LogLevel::Info,
            });
        }
        let snap = lb.snapshot();
        assert_eq!(snap.len(), 4);
        assert_eq!(snap[0].text, "e6");
        assert_eq!(snap[3].text, "e9");
    }
}
