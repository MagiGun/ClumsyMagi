//! Win32 platform helpers: UAC elevation, admin checks, single-instance guard,
//! high-resolution timer period, message boxes. Hand-rolled FFI to keep the
//! dependency footprint minimal.

#![cfg(windows)]
#![allow(non_snake_case)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;

pub fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------- elevation

#[link(name = "kernel32")]
extern "system" {
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: *mut c_void, desired_access: u32, token: *mut *mut c_void) -> i32;
    fn GetTokenInformation(
        token: *mut c_void,
        info_class: u32,
        info: *mut c_void,
        len: u32,
        ret_len: *mut u32,
    ) -> i32;
}

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_ELEVATION: u32 = 20;

/// Is the current process token elevated? (elevate.c IsElevated)
pub fn is_elevated() -> bool {
    unsafe {
        let mut token: *mut c_void = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: u32 = 0;
        let mut ret: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TOKEN_ELEVATION,
            &mut elevation as *mut u32 as *mut c_void,
            4,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elevation != 0
    }
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(info: *mut SHELLEXECUTEINFOW) -> i32;
}

#[repr(C)]
struct SHELLEXECUTEINFOW {
    cbSize: u32,
    fMask: u32,
    hwnd: *mut c_void,
    lpVerb: *const u16,
    lpFile: *const u16,
    lpParameters: *const u16,
    lpDirectory: *const u16,
    nShow: i32,
    hInstApp: *mut c_void,
    lpIDList: *mut c_void,
    lpClass: *const u16,
    hkeyClass: *mut c_void,
    dwHotKey: u32,
    hIconOrMonitor: *mut c_void,
    hProcess: *mut c_void,
}

const SEE_MASK_DEFAULT: u32 = 0;
const SW_NORMAL: i32 = 1;

/// Relaunch this executable elevated via the "runas" shell verb (UAC prompt).
/// Returns Err on user-cancel or failure.
pub fn relaunch_elevated(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to get exe path: {e}"))?;
    let exe_w = wide(&exe.to_string_lossy());
    let params = args.join(" ");
    let params_w = if params.is_empty() {
        vec![0u16]
    } else {
        wide(&params)
    };
    let verb_w = wide("runas");

    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_DEFAULT,
        hwnd: std::ptr::null_mut(),
        lpVerb: verb_w.as_ptr(),
        lpFile: exe_w.as_ptr(),
        lpParameters: params_w.as_ptr(),
        lpDirectory: std::ptr::null(),
        nShow: SW_NORMAL,
        hInstApp: std::ptr::null_mut(),
        lpIDList: std::ptr::null_mut(),
        lpClass: std::ptr::null(),
        hkeyClass: std::ptr::null_mut(),
        dwHotKey: 0,
        hIconOrMonitor: std::ptr::null_mut(),
        hProcess: std::ptr::null_mut(),
    };
    let ok = unsafe { ShellExecuteExW(&mut sei) };
    if ok == 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == 1223 {
            // ERROR_CANCELLED
            return Err("Elevation was cancelled. ClumsyMagi needs Administrator rights to capture packets.".to_string());
        }
        return Err(format!("Failed to elevate (error {code}). Right-click the exe and choose 'Run as Administrator'."));
    }
    Ok(())
}

// ---------------------------------------------------------------- single instance

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(attr: *mut c_void, initial_owner: i32, name: *const u16) -> *mut c_void;
    fn ReleaseMutex(handle: *mut c_void) -> i32;
}

/// Global named-mutex guard; mirrors clumsy's "Global\\CLUMSY_IS_RUNNING_EVENT_NAME".
pub struct SingleInstance {
    handle: *mut c_void,
}

impl SingleInstance {
    /// Returns None when another instance is already running.
    pub fn acquire() -> Option<SingleInstance> {
        let name = wide("Global\\CLUMSYMAGI_IS_RUNNING");
        let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Some(SingleInstance { handle }); // failed to create; allow running
        }
        // ERROR_ALREADY_EXISTS (183)?
        if std::io::Error::last_os_error().raw_os_error() == Some(183) {
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(SingleInstance { handle })
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                ReleaseMutex(self.handle);
                CloseHandle(self.handle);
            }
        }
    }
}

// ---------------------------------------------------------------- timer resolution

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}

const TIMER_RESOLUTION: u32 = 4;

pub fn start_timer_resolution() {
    unsafe {
        timeBeginPeriod(TIMER_RESOLUTION);
    }
}

pub fn end_timer_resolution() {
    unsafe {
        timeEndPeriod(TIMER_RESOLUTION);
    }
}

// ---------------------------------------------------------------- message box

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hwnd: *mut c_void, text: *const u16, caption: *const u16, flags: u32) -> i32;
}

const MB_OK: u32 = 0x0;
const MB_ICONERROR: u32 = 0x10;

/// Blocking error dialog for startup failures (used before the GUI exists).
pub fn error_box(text: &str) {
    let text_w = wide(text);
    let caption_w = wide("ClumsyMagi");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text_w.as_ptr(),
            caption_w.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
