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

// ---------------------------------------------------------------- system theme detection

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        hKey: *mut c_void,
        lpSubKey: *const u16,
        ulOptions: u32,
        samDesired: u32,
        phkResult: *mut *mut c_void,
    ) -> i32;
    fn RegQueryValueExW(
        hKey: *mut c_void,
        lpValueName: *const u16,
        lpReserved: *mut u32,
        lpType: *mut u32,
        lpData: *mut u8,
        lpcbData: *mut u32,
    ) -> i32;
    fn RegCloseKey(hKey: *mut c_void) -> i32;
}

const HKEY_CURRENT_USER: *mut c_void = ((0x80000001u32 as i32) as isize) as usize as *mut c_void;
const KEY_QUERY_VALUE: u32 = 0x0001;

/// Queries Windows registry to check if the user has Dark Mode configured for apps.
/// HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme
/// Returns true if dark mode is active (AppsUseLightTheme == 0), or true on failure/missing key.
pub fn is_system_dark_mode() -> bool {
    let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let val_name = wide("AppsUseLightTheme");
    unsafe {
        let mut hkey: *mut c_void = std::ptr::null_mut();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        );
        if status != 0 || hkey.is_null() {
            return true;
        }

        let mut val: u32 = 0;
        let mut val_len: u32 = std::mem::size_of::<u32>() as u32;
        let mut val_type: u32 = 0;
        let query_status = RegQueryValueExW(
            hkey,
            val_name.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            &mut val as *mut u32 as *mut u8,
            &mut val_len,
        );
        RegCloseKey(hkey);

        if query_status == 0 {
            val == 0
        } else {
            true
        }
    }
}

// ---------------------------------------------------------------- console management

#[link(name = "kernel32")]
extern "system" {
    fn GetConsoleWindow() -> *mut c_void;
    fn AllocConsole() -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn ShowWindow(hWnd: *mut c_void, nCmdShow: i32) -> i32;
    fn IsWindowVisible(hWnd: *mut c_void) -> i32;
    fn GetSystemMenu(hWnd: *mut c_void, bRevert: i32) -> *mut c_void;
    fn EnableMenuItem(hMenu: *mut c_void, uIDEnableItem: u32, uEnable: u32) -> i32;
}

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;
const SC_CLOSE: u32 = 0xF060;
const MF_BYCOMMAND: u32 = 0x00000000;
const MF_GRAYED: u32 = 0x00000001;
const MF_DISABLED: u32 = 0x00000002;

/// Hide the console window if one is attached to this process.
pub fn hide_console() {
    unsafe {
        let hwnd = GetConsoleWindow();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// Show the console window (allocating one if not present) and disable its Close
/// button so accidental closes don't kill the application process.
pub fn show_console() {
    unsafe {
        let mut hwnd = GetConsoleWindow();
        if hwnd.is_null() {
            AllocConsole();
            hwnd = GetConsoleWindow();
        }
        if !hwnd.is_null() {
            let menu = GetSystemMenu(hwnd, 0);
            if !menu.is_null() {
                EnableMenuItem(menu, SC_CLOSE, MF_BYCOMMAND | MF_GRAYED | MF_DISABLED);
            }
            ShowWindow(hwnd, SW_SHOW);
        }
    }
}

/// Is the console window currently visible?
pub fn is_console_visible() -> bool {
    unsafe {
        let hwnd = GetConsoleWindow();
        if hwnd.is_null() {
            false
        } else {
            IsWindowVisible(hwnd) != 0
        }
    }
}

/// Toggle console visibility; returns true if now visible, false if now hidden.
pub fn toggle_console() -> bool {
    if is_console_visible() {
        hide_console();
        false
    } else {
        show_console();
        true
    }
}

// ---------------------------------------------------------------- screen & window centering

pub const SPI_GETWORKAREA: u32 = 0x0030;
pub const SM_CXSCREEN: i32 = 0;
pub const SM_CYSCREEN: i32 = 1;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct MONITORINFO {
    pub cbSize: u32,
    pub rcMonitor: RECT,
    pub rcWork: RECT,
    pub dwFlags: u32,
}

pub const MONITOR_DEFAULTTOPRIMARY: u32 = 0x00000001;

#[link(name = "user32")]
extern "system" {
    fn SystemParametersInfoW(
        uiAction: u32,
        uiParam: u32,
        pvParam: *mut c_void,
        fWinIni: u32,
    ) -> i32;
    fn GetSystemMetrics(nIndex: i32) -> i32;
    fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    fn MonitorFromPoint(pt: POINT, dwFlags: u32) -> *mut c_void;
    fn GetMonitorInfoW(hMonitor: *mut c_void, lpmi: *mut MONITORINFO) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(lpModuleName: *const u16) -> *mut c_void;
    fn GetProcAddress(hModule: *mut c_void, lpProcName: *const u8) -> *mut c_void;
}

/// Dynamically query GetDpiForSystem from user32.dll (supported on Win10 1607+),
/// gracefully falling back to standard 96 DPI (100% scale) on older systems.
pub fn get_system_dpi() -> u32 {
    unsafe {
        let user32 = wide("user32.dll");
        let hmod = GetModuleHandleW(user32.as_ptr());
        if !hmod.is_null() {
            let proc_name = b"GetDpiForSystem\0";
            let proc = GetProcAddress(hmod, proc_name.as_ptr());
            if !proc.is_null() {
                let func: extern "system" fn() -> u32 = std::mem::transmute(proc);
                let dpi = func();
                if dpi > 0 {
                    return dpi;
                }
            }
        }
        96
    }
}

/// Returns the active monitor's work area rectangle (excluding taskbars / appbars) in pixels.
/// Detects the monitor containing the mouse cursor (or primary display monitor),
/// falling back to SystemParametersInfoW / GetSystemMetrics if unavailable.
pub fn get_active_work_area() -> RECT {
    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) } != 0 {
        let hmon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY) };
        if !hmon.is_null() {
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: RECT::default(),
                rcWork: RECT::default(),
                dwFlags: 0,
            };
            if unsafe { GetMonitorInfoW(hmon, &mut mi) } != 0 {
                if mi.rcWork.right > mi.rcWork.left && mi.rcWork.bottom > mi.rcWork.top {
                    return mi.rcWork;
                }
            }
        }
    }

    let mut rect = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            &mut rect as *mut RECT as *mut c_void,
            0,
        )
    };
    if ok != 0 && rect.right > rect.left && rect.bottom > rect.top {
        rect
    } else {
        let w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        RECT {
            left: 0,
            top: 0,
            right: if w > 0 { w } else { 1920 },
            bottom: if h > 0 { h } else { 1080 },
        }
    }
}

/// Calculates the centered position [x, y] for a window of the given size in logical points,
/// taking into account the active work area (avoiding taskbar obstruction) and system DPI scale.
pub fn get_centered_window_position(width: f32, height: f32) -> [f32; 2] {
    let dpi = get_system_dpi();
    let scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };

    let work = get_active_work_area();
    let work_left = work.left as f32 / scale;
    let work_top = work.top as f32 / scale;
    let work_w = (work.right - work.left).max(1) as f32 / scale;
    let work_h = (work.bottom - work.top).max(1) as f32 / scale;

    let x = work_left + ((work_w - width) * 0.5).max(0.0);
    let y = work_top + ((work_h - height) * 0.5).max(0.0);
    [x, y]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_dark_mode_readable() {
        // Must succeed without crashing and return a valid boolean
        let is_dark = is_system_dark_mode();
        println!("is_system_dark_mode() = {is_dark}");
        // On user's machine AppsUseLightTheme is 0, so is_dark should be true
        assert!(is_dark);
    }

    #[test]
    fn test_work_area_and_centering() {
        let dpi = get_system_dpi();
        println!("System DPI: {dpi} (scale: {})", dpi as f32 / 96.0);
        let rect = get_active_work_area();
        println!("Active work area: {rect:?}");
        assert!(rect.right > rect.left);
        assert!(rect.bottom > rect.top);

        let [x, y] = get_centered_window_position(580.0, 680.0);
        println!("Centered pos for 580x680: x={x}, y={y}");
        assert!(x >= 0.0);
        assert!(y >= 0.0);
    }
}

