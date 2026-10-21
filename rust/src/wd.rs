//! WinDivert 2.2 dynamic FFI layer.
//!
//! WinDivert.dll is loaded at runtime (not linked) so we can produce friendly
//! errors when it is missing or fails to install its driver. The library is
//! loaded once per process and intentionally never unloaded (WinDivert does not
//! support safe unloading), so all extracted function pointers stay valid.
//!
//! The `WINDIVERT_ADDRESS` struct is mirrored by hand: the C bitfields are
//! packed LSB-first on MSVC x64, we encode/decode them explicitly.

#![allow(non_snake_case)]

use libloading::Library;
use std::ffi::c_void;
use std::path::PathBuf;

pub const WINDIVERT_LAYER_NETWORK: i32 = 0;
pub const WINDIVERT_PARAM_QUEUE_LENGTH: i32 = 0;
pub const WINDIVERT_PARAM_QUEUE_TIME: i32 = 1;
pub const WINDIVERT_SHUTDOWN_BOTH: i32 = 3;

pub const MAX_PACKETSIZE: usize = 0xFFFF;

// WINDIVERT_ADDRESS layout: i64 timestamp, u32 bitfield word, u32 reserved,
// then a 64-byte union (Network data is the first 8 bytes).
pub const WINDIVERT_ADDRESS_SIZE: usize = 80;
pub const WINDIVERT_DATA_UNION_OFFSET: usize = 16;
pub const WINDIVERT_DATA_UNION_SIZE: usize = 64;

/// Rust-side mirror of `WINDIVERT_ADDRESS`. The raw union bytes are preserved
/// and passed back on re-injection, so non-network layers stay transparent.
#[derive(Clone, Debug)]
pub struct WdAddress {
    pub timestamp: i64,
    pub layer: u8,
    pub event: u8,
    pub sniffed: bool,
    pub outbound: bool,
    pub loopback: bool,
    pub impostor: bool,
    pub ipv6: bool,
    pub ip_checksum: bool,
    pub tcp_checksum: bool,
    pub udp_checksum: bool,
    pub data: [u8; WINDIVERT_DATA_UNION_SIZE],
}

impl Default for WdAddress {
    fn default() -> Self {
        WdAddress {
            timestamp: 0,
            layer: 0,
            event: 0,
            sniffed: false,
            outbound: false,
            loopback: false,
            impostor: false,
            ipv6: false,
            ip_checksum: false,
            tcp_checksum: false,
            udp_checksum: false,
            data: [0u8; WINDIVERT_DATA_UNION_SIZE],
        }
    }
}

impl WdAddress {
    pub fn from_raw(buf: &[u8; WINDIVERT_ADDRESS_SIZE]) -> WdAddress {
        let ts = i64::from_le_bytes(buf[0..8].try_into().unwrap());
        let flags = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let mut data = [0u8; WINDIVERT_DATA_UNION_SIZE];
        data.copy_from_slice(&buf[WINDIVERT_DATA_UNION_OFFSET..]);
        WdAddress {
            timestamp: ts,
            layer: (flags & 0xFF) as u8,
            event: ((flags >> 8) & 0xFF) as u8,
            sniffed: flags & (1 << 16) != 0,
            outbound: flags & (1 << 17) != 0,
            loopback: flags & (1 << 18) != 0,
            impostor: flags & (1 << 19) != 0,
            ipv6: flags & (1 << 20) != 0,
            ip_checksum: flags & (1 << 21) != 0,
            tcp_checksum: flags & (1 << 22) != 0,
            udp_checksum: flags & (1 << 23) != 0,
            data,
        }
    }

    pub fn to_raw(&self, out: &mut [u8; WINDIVERT_ADDRESS_SIZE]) {
        *out = [0u8; WINDIVERT_ADDRESS_SIZE];
        out[0..8].copy_from_slice(&self.timestamp.to_le_bytes());
        let mut flags: u32 = self.layer as u32 | (self.event as u32) << 8;
        for (bit, on) in [
            (16, self.sniffed),
            (17, self.outbound),
            (18, self.loopback),
            (19, self.impostor),
            (20, self.ipv6),
            (21, self.ip_checksum),
            (22, self.tcp_checksum),
            (23, self.udp_checksum),
        ] {
            if on {
                flags |= 1 << bit;
            }
        }
        out[8..12].copy_from_slice(&flags.to_le_bytes());
        out[WINDIVERT_DATA_UNION_OFFSET..].copy_from_slice(&self.data);
    }
}

type WinDivertOpenFn =
    unsafe extern "system" fn(filter: *const u8, layer: i32, priority: i16, flags: u64) -> *mut c_void;
type WinDivertCloseFn = unsafe extern "system" fn(handle: *mut c_void) -> i32;
type WinDivertRecvFn = unsafe extern "system" fn(
    handle: *mut c_void,
    packet: *mut u8,
    packet_len: u32,
    recv_len: *mut u32,
    addr: *mut u8,
) -> i32;
type WinDivertSendFn = unsafe extern "system" fn(
    handle: *mut c_void,
    packet: *mut u8,
    packet_len: u32,
    send_len: *mut u32,
    addr: *const u8,
) -> i32;
type WinDivertShutdownFn = unsafe extern "system" fn(handle: *mut c_void, how: i32) -> i32;
type WinDivertSetParamFn = unsafe extern "system" fn(handle: *mut c_void, param: i32, value: u64) -> i32;
type WinDivertHelperCompileFilterFn = unsafe extern "system" fn(
    filter: *const u8,
    layer: i32,
    object: *mut u8,
    obj_len: u32,
    error_str: *mut *const u8,
    error_pos: *mut u32,
) -> i32;

/// The loaded WinDivert.dll. Lives for the whole process.
pub struct WinDivertLib {
    open_fn: WinDivertOpenFn,
    close_fn: WinDivertCloseFn,
    recv_fn: WinDivertRecvFn,
    send_fn: WinDivertSendFn,
    shutdown_fn: WinDivertShutdownFn,
    set_param_fn: WinDivertSetParamFn,
    compile_filter_fn: WinDivertHelperCompileFilterFn,
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum WdError {
    /// WinDivert.dll could not be found/loaded.
    DllLoad(String),
    /// WinDivertOpen failed; carries the Win32 error code and a friendly hint.
    Open(u32, String),
    /// WinDivertRecv/Send failed with the given Win32 error code.
    Io(u32),
}

impl WdError {
    pub fn friendly(&self) -> String {
        match self {
            WdError::DllLoad(e) => format!(
                "Failed to load WinDivert.dll: {e}\nMake sure WinDivert.dll is placed next to clumsy.exe."
            ),
            WdError::Open(code, hint) => format!("{hint} (Win32 error {code})"),
            WdError::Io(code) => format!("WinDivert I/O error {code}"),
        }
    }
}

fn last_error() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
}

fn open_error(code: u32) -> WdError {
    let hint = match code {
        2 => "WinDivert driver files not found. Make sure WinDivert.dll and the .sys driver are next to the executable.".to_string(),
        3 => "Failed to load the WinDivert driver. An old WinDivert service may be stuck.\nTry:  sc stop WinDivert\n      sc delete WinDivert\nthen retry (a reboot may be required).".to_string(),
        5 => "Access denied. ClumsyMagi must be run as Administrator to capture packets.".to_string(),
        87 => "Filter syntax error.".to_string(),
        577 => "WinDivert driver could not be loaded (invalid image hash / driver signing issue).".to_string(),
        1058 | 1064 => "WinDivert service is disabled or failed to start.\nTry:  sc stop WinDivert\n      sc delete WinDivert\nthen retry.".to_string(),
        _ => "Failed to open the WinDivert device. Make sure you are running as Administrator.".to_string(),
    };
    WdError::Open(code, hint)
}

impl WinDivertLib {
    /// Locate and load WinDivert.dll: searched next to the exe, then the DLL
    /// search path. Intentionally leaked (never unloaded).
    pub fn load() -> Result<WinDivertLib, WdError> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("WinDivert.dll"));
            }
        }
        candidates.push(PathBuf::from("WinDivert.dll"));

        let mut last_err = String::from("WinDivert.dll not found");
        let mut loaded: Option<(PathBuf, Library)> = None;
        for path in &candidates {
            if path.exists() {
                match unsafe { Library::new(path) } {
                    Ok(lib) => {
                        loaded = Some((path.clone(), lib));
                        break;
                    }
                    Err(e) => last_err = format!("{e}"),
                }
            }
        }
        let (path, lib) = loaded.ok_or_else(|| WdError::DllLoad(last_err))?;

        // Extract everything we need up-front; leak the library afterwards so
        // the pointers stay valid for the process lifetime.
        let open_fn = get::<WinDivertOpenFn>(&lib, b"WinDivertOpen")?;
        let close_fn = get::<WinDivertCloseFn>(&lib, b"WinDivertClose")?;
        let recv_fn = get::<WinDivertRecvFn>(&lib, b"WinDivertRecv")?;
        let send_fn = get::<WinDivertSendFn>(&lib, b"WinDivertSend")?;
        let shutdown_fn = get::<WinDivertShutdownFn>(&lib, b"WinDivertShutdown")?;
        let set_param_fn = get::<WinDivertSetParamFn>(&lib, b"WinDivertSetParam")?;
        let compile_filter_fn = get::<WinDivertHelperCompileFilterFn>(&lib, b"WinDivertHelperCompileFilter")?;
        std::mem::forget(lib); // never unload WinDivert

        Ok(WinDivertLib {
            open_fn,
            close_fn,
            recv_fn,
            send_fn,
            shutdown_fn,
            set_param_fn,
            compile_filter_fn,
            path,
        })
    }

    /// Validate a filter string using WinDivert's own compiler (2.2 signature:
    /// error string is a pointer into WinDivert's static buffer).
    pub fn compile_filter(&self, filter: &str) -> Result<(), String> {
        let mut wide = to_cstring(filter);
        let mut err_ptr: *const u8 = std::ptr::null();
        let mut err_pos: u32 = 0;
        let ok = unsafe {
            (self.compile_filter_fn)(
                wide.as_mut_ptr(),
                WINDIVERT_LAYER_NETWORK,
                std::ptr::null_mut(),
                0,
                &mut err_ptr,
                &mut err_pos,
            )
        };
        if ok != 0 {
            return Ok(());
        }
        let msg = if err_ptr.is_null() {
            String::new()
        } else {
            unsafe {
                let mut len = 0usize;
                while *err_ptr.add(len) != 0 {
                    len += 1;
                }
                String::from_utf8_lossy(std::slice::from_raw_parts(err_ptr, len))
                    .trim()
                    .to_string()
            }
        };
        if msg.is_empty() {
            Err(format!("Filter syntax error at position {err_pos}"))
        } else {
            Err(format!("Filter syntax error: {msg}"))
        }
    }

    /// Open a WinDivert handle for the NETWORK layer at priority 0.
    /// The caller owns the raw handle (see the *_on methods).
    pub fn open(&self, filter: &str) -> Result<*mut c_void, WdError> {
        let mut wide = to_cstring(filter);
        let handle = unsafe { (self.open_fn)(wide.as_mut_ptr(), WINDIVERT_LAYER_NETWORK, 0, 0) };
        if handle.is_null() || handle as isize == -1 {
            return Err(open_error(last_error()));
        }
        Ok(handle)
    }

    pub fn recv_on(
        &self,
        handle: *mut c_void,
        buf: &mut [u8],
        addr_out: &mut [u8; WINDIVERT_ADDRESS_SIZE],
    ) -> Result<usize, WdError> {
        let mut recv_len: u32 = 0;
        let ok = unsafe {
            (self.recv_fn)(handle, buf.as_mut_ptr(), buf.len() as u32, &mut recv_len, addr_out.as_mut_ptr())
        };
        if ok == 0 {
            return Err(WdError::Io(last_error()));
        }
        Ok(recv_len as usize)
    }

    pub fn send_on(&self, handle: *mut c_void, packet: &mut [u8], addr: &WdAddress) -> Result<usize, WdError> {
        let mut raw = [0u8; WINDIVERT_ADDRESS_SIZE];
        addr.to_raw(&mut raw);
        let mut send_len: u32 = 0;
        let ok = unsafe {
            (self.send_fn)(handle, packet.as_mut_ptr(), packet.len() as u32, &mut send_len, raw.as_ptr())
        };
        if ok == 0 {
            return Err(WdError::Io(last_error()));
        }
        Ok(send_len as usize)
    }

    pub fn set_param_on(&self, handle: *mut c_void, param: i32, value: u64) {
        unsafe {
            (self.set_param_fn)(handle, param, value);
        }
    }

    /// Unblock all pending Recv calls (used to stop the recv thread).
    pub fn shutdown(&self, handle: *mut c_void) {
        unsafe {
            (self.shutdown_fn)(handle, WINDIVERT_SHUTDOWN_BOTH);
        }
    }

    /// Close a raw handle. After this the handle must not be used again.
    pub fn close_on(&self, handle: *mut c_void) {
        unsafe {
            (self.close_fn)(handle);
        }
    }
}

unsafe impl Send for WinDivertLib {}
unsafe impl Sync for WinDivertLib {}

fn get<T: Copy>(lib: &Library, name: &[u8]) -> Result<T, WdError> {
    unsafe {
        lib.get::<T>(name)
            .map(|s| *s)
            .map_err(|e| WdError::DllLoad(format!("missing export {}: {e}", String::from_utf8_lossy(name))))
    }
}

/// RAII wrapper over a raw handle, for simple single-owner usage (e.g. selftest).
#[allow(dead_code)]
pub struct WinDivertGuard<'a> {
    lib: &'a WinDivertLib,
    handle: *mut c_void,
}

#[allow(dead_code)]
unsafe impl Send for WinDivertGuard<'_> {}

#[allow(dead_code)]
impl<'a> WinDivertGuard<'a> {
    pub fn open(lib: &'a WinDivertLib, filter: &str) -> Result<WinDivertGuard<'a>, WdError> {
        let handle = lib.open(filter)?;
        Ok(WinDivertGuard { lib, handle })
    }

    pub fn raw(&self) -> *mut c_void {
        self.handle
    }

    pub fn set_param(&self, param: i32, value: u64) {
        self.lib.set_param_on(self.handle, param, value);
    }

    pub fn recv(&self, buf: &mut [u8], addr_out: &mut [u8; WINDIVERT_ADDRESS_SIZE]) -> Result<usize, WdError> {
        self.lib.recv_on(self.handle, buf, addr_out)
    }

    pub fn send(&self, packet: &mut [u8], addr: &WdAddress) -> Result<usize, WdError> {
        self.lib.send_on(self.handle, packet, addr)
    }
}

#[allow(dead_code)]
impl Drop for WinDivertGuard<'_> {
    fn drop(&mut self) {
        self.lib.close_on(self.handle);
    }
}

fn to_cstring(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}
