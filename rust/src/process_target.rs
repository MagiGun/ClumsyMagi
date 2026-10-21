//! Process & Window target selector for ClumsyMagi.
//! Allows users to select any running game, window, or application and
//! immediately intercept and control its network packets (e.g. adding lag).

#![cfg(windows)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::net::{Ipv4Addr, Ipv6Addr};

// ---------------------------------------------------------------- Data models

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSocket {
    pub protocol: SocketProtocol,
    pub local_port: u16,
    pub remote_port: u16,
    pub remote_addr: String,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub window_title: Option<String>,
    pub sockets: Vec<ProcessSocket>,
    pub is_likely_game: bool,
}

// ---------------------------------------------------------------- IP Helper API (Sockets)

#[link(name = "iphlpapi")]
extern "system" {
    fn GetExtendedTcpTable(
        pTcpTable: *mut c_void,
        pdwSize: *mut u32,
        bOrder: i32,
        ulAf: u32,
        TableClass: i32,
        Reserved: u32,
    ) -> u32;

    fn GetExtendedUdpTable(
        pUdpTable: *mut c_void,
        pdwSize: *mut u32,
        bOrder: i32,
        ulAf: u32,
        TableClass: i32,
        Reserved: u32,
    ) -> u32;
}

const AF_INET: u32 = 2;
const AF_INET6: u32 = 23;
const TCP_TABLE_OWNER_PID_ALL: i32 = 5;
const UDP_TABLE_OWNER_PID: i32 = 1;
const NO_ERROR: u32 = 0;

#[repr(C)]
struct MibTcpRowOwnerPid {
    dw_state: u32,
    dw_local_addr: u32,
    dw_local_port: u32,
    dw_remote_addr: u32,
    dw_remote_port: u32,
    dw_owning_pid: u32,
}

#[repr(C)]
struct MibTcp6RowOwnerPid {
    uc_local_addr: [u8; 16],
    dw_local_scope_id: u32,
    dw_local_port: u32,
    uc_remote_addr: [u8; 16],
    dw_remote_scope_id: u32,
    dw_remote_port: u32,
    dw_state: u32,
    dw_owning_pid: u32,
}

#[repr(C)]
struct MibUdpRowOwnerPid {
    dw_local_addr: u32,
    dw_local_port: u32,
    dw_owning_pid: u32,
}

#[repr(C)]
struct MibUdp6RowOwnerPid {
    uc_local_addr: [u8; 16],
    dw_local_scope_id: u32,
    dw_local_port: u32,
    dw_owning_pid: u32,
}

fn tcp_state_name(state: u32) -> &'static str {
    match state {
        1 => "CLOSED",
        2 => "LISTEN",
        3 => "SYN_SENT",
        4 => "SYN_RCVD",
        5 => "ESTABLISHED",
        6 => "FIN_WAIT1",
        7 => "FIN_WAIT2",
        8 => "CLOSE_WAIT",
        9 => "CLOSING",
        10 => "LAST_ACK",
        11 => "TIME_WAIT",
        12 => "DELETE_TCB",
        _ => "UNKNOWN",
    }
}

pub fn get_all_process_sockets() -> HashMap<u32, Vec<ProcessSocket>> {
    let mut map: HashMap<u32, Vec<ProcessSocket>> = HashMap::new();

    // 1. TCP IPv4
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0);
        if size > 0 {
            let mut buf: Vec<u8> = vec![0; size as usize];
            if GetExtendedTcpTable(buf.as_mut_ptr() as *mut c_void, &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0) == NO_ERROR {
                let num_entries = *(buf.as_ptr() as *const u32) as usize;
                let rows_ptr = buf.as_ptr().add(4) as *const MibTcpRowOwnerPid;
                for i in 0..num_entries {
                    let row = &*rows_ptr.add(i);
                    let local_port = u16::from_be((row.dw_local_port & 0xFFFF) as u16);
                    let remote_port = u16::from_be((row.dw_remote_port & 0xFFFF) as u16);
                    let remote_addr = Ipv4Addr::from(u32::from_be(row.dw_remote_addr)).to_string();
                    map.entry(row.dw_owning_pid).or_default().push(ProcessSocket {
                        protocol: SocketProtocol::Tcp,
                        local_port,
                        remote_port,
                        remote_addr,
                        state: tcp_state_name(row.dw_state).to_string(),
                    });
                }
            }
        }
    }

    // 2. TCP IPv6
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET6, TCP_TABLE_OWNER_PID_ALL, 0);
        if size > 0 {
            let mut buf: Vec<u8> = vec![0; size as usize];
            if GetExtendedTcpTable(buf.as_mut_ptr() as *mut c_void, &mut size, 0, AF_INET6, TCP_TABLE_OWNER_PID_ALL, 0) == NO_ERROR {
                let num_entries = *(buf.as_ptr() as *const u32) as usize;
                let rows_ptr = buf.as_ptr().add(4) as *const MibTcp6RowOwnerPid;
                for i in 0..num_entries {
                    let row = &*rows_ptr.add(i);
                    let local_port = u16::from_be((row.dw_local_port & 0xFFFF) as u16);
                    let remote_port = u16::from_be((row.dw_remote_port & 0xFFFF) as u16);
                    let remote_addr = Ipv6Addr::from(row.uc_remote_addr).to_string();
                    map.entry(row.dw_owning_pid).or_default().push(ProcessSocket {
                        protocol: SocketProtocol::Tcp,
                        local_port,
                        remote_port,
                        remote_addr,
                        state: tcp_state_name(row.dw_state).to_string(),
                    });
                }
            }
        }
    }

    // 3. UDP IPv4
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedUdpTable(std::ptr::null_mut(), &mut size, 0, AF_INET, UDP_TABLE_OWNER_PID, 0);
        if size > 0 {
            let mut buf: Vec<u8> = vec![0; size as usize];
            if GetExtendedUdpTable(buf.as_mut_ptr() as *mut c_void, &mut size, 0, AF_INET, UDP_TABLE_OWNER_PID, 0) == NO_ERROR {
                let num_entries = *(buf.as_ptr() as *const u32) as usize;
                let rows_ptr = buf.as_ptr().add(4) as *const MibUdpRowOwnerPid;
                for i in 0..num_entries {
                    let row = &*rows_ptr.add(i);
                    let local_port = u16::from_be((row.dw_local_port & 0xFFFF) as u16);
                    map.entry(row.dw_owning_pid).or_default().push(ProcessSocket {
                        protocol: SocketProtocol::Udp,
                        local_port,
                        remote_port: 0,
                        remote_addr: String::new(),
                        state: "UDP".to_string(),
                    });
                }
            }
        }
    }

    // 4. UDP IPv6
    unsafe {
        let mut size: u32 = 0;
        let _ = GetExtendedUdpTable(std::ptr::null_mut(), &mut size, 0, AF_INET6, UDP_TABLE_OWNER_PID, 0);
        if size > 0 {
            let mut buf: Vec<u8> = vec![0; size as usize];
            if GetExtendedUdpTable(buf.as_mut_ptr() as *mut c_void, &mut size, 0, AF_INET6, UDP_TABLE_OWNER_PID, 0) == NO_ERROR {
                let num_entries = *(buf.as_ptr() as *const u32) as usize;
                let rows_ptr = buf.as_ptr().add(4) as *const MibUdp6RowOwnerPid;
                for i in 0..num_entries {
                    let row = &*rows_ptr.add(i);
                    let local_port = u16::from_be((row.dw_local_port & 0xFFFF) as u16);
                    map.entry(row.dw_owning_pid).or_default().push(ProcessSocket {
                        protocol: SocketProtocol::Udp,
                        local_port,
                        remote_port: 0,
                        remote_addr: String::new(),
                        state: "UDP".to_string(),
                    });
                }
            }
        }
    }

    // Deduplicate identical sockets per PID
    for sockets in map.values_mut() {
        sockets.dedup_by(|a, b| {
            a.protocol == b.protocol
                && a.local_port == b.local_port
                && a.remote_port == b.remote_port
                && a.remote_addr == b.remote_addr
        });
    }

    map
}

// ---------------------------------------------------------------- Toolhelp32 & Windows

#[repr(C)]
struct PROCESSENTRY32W {
    dwSize: u32,
    cntUsage: u32,
    th32ProcessID: u32,
    th32DefaultHeapID: usize,
    th32ModuleID: u32,
    cntThreads: u32,
    th32ParentProcessID: u32,
    pcPriClassBase: i32,
    dwFlags: u32,
    szExeFile: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> *mut c_void;
    fn Process32FirstW(hSnapshot: *mut c_void, lppe: *mut PROCESSENTRY32W) -> i32;
    fn Process32NextW(hSnapshot: *mut c_void, lppe: *mut PROCESSENTRY32W) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

const TH32CS_SNAPPROCESS: u32 = 0x00000002;
const INVALID_HANDLE_VALUE: *mut c_void = -1isize as usize as *mut c_void;

type WNDENUMPROC = unsafe extern "system" fn(hwnd: *mut c_void, lParam: isize) -> i32;

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(lpEnumFunc: WNDENUMPROC, lParam: isize) -> i32;
    fn GetWindowThreadProcessId(hWnd: *mut c_void, lpdwProcessId: *mut u32) -> u32;
    fn IsWindowVisible(hWnd: *mut c_void) -> i32;
    fn GetWindowTextLengthW(hWnd: *mut c_void) -> i32;
    fn GetWindowTextW(hWnd: *mut c_void, lpString: *mut u16, nMaxCount: i32) -> i32;
}

unsafe extern "system" fn enum_windows_callback(hwnd: *mut c_void, lparam: isize) -> i32 {
    let map = &mut *(lparam as *mut HashMap<u32, String>);
    if IsWindowVisible(hwnd) != 0 {
        let len = GetWindowTextLengthW(hwnd);
        if len > 0 {
            let mut title_buf: Vec<u16> = vec![0; (len + 1) as usize];
            let read_len = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), len + 1);
            if read_len > 0 {
                let title = String::from_utf16_lossy(&title_buf[..read_len as usize]).trim().to_string();
                if !title.is_empty() {
                    let mut pid: u32 = 0;
                    GetWindowThreadProcessId(hwnd, &mut pid);
                    if pid != 0 {
                        map.entry(pid).or_insert(title);
                    }
                }
            }
        }
    }
    1
}

pub fn get_window_titles() -> HashMap<u32, String> {
    let mut map: HashMap<u32, String> = HashMap::new();
    unsafe {
        EnumWindows(enum_windows_callback, &mut map as *mut _ as isize);
    }
    map
}

/// Lists all running processes, associating each with any visible window titles
/// and active network sockets (TCP & UDP).
pub fn list_processes() -> Vec<ProcessInfo> {
    let mut sockets_by_pid = get_all_process_sockets();
    let mut window_titles = get_window_titles();

    let mut result = Vec::new();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() || snapshot == INVALID_HANDLE_VALUE {
        return result;
    }

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        cntUsage: 0,
        th32ProcessID: 0,
        th32DefaultHeapID: 0,
        th32ModuleID: 0,
        cntThreads: 0,
        th32ParentProcessID: 0,
        pcPriClassBase: 0,
        dwFlags: 0,
        szExeFile: [0; 260],
    };

    let mut success = unsafe { Process32FirstW(snapshot, &mut entry) != 0 };
    while success {
        let pid = entry.th32ProcessID;
        if pid != 0 {
            let name_len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..name_len]);

            let window_title = window_titles.remove(&pid);
            let sockets = sockets_by_pid.remove(&pid).unwrap_or_default();

            // Exclude common noise from "likely game" flag
            let lower_name = name.to_ascii_lowercase();
            let is_system_noise = lower_name == "system"
                || lower_name == "svchost.exe"
                || lower_name == "csrss.exe"
                || lower_name == "services.exe"
                || lower_name == "lsass.exe"
                || lower_name == "smss.exe"
                || lower_name == "dwm.exe"
                || lower_name == "explorer.exe";

            let is_likely_game = !is_system_noise && (window_title.is_some() || !sockets.is_empty());

            result.push(ProcessInfo {
                pid,
                name,
                window_title,
                sockets,
                is_likely_game,
            });
        }

        success = unsafe { Process32NextW(snapshot, &mut entry) != 0 };
    }

    unsafe { CloseHandle(snapshot) };

    // Sort: likely games / active windowed apps with network sockets first, then by name
    result.sort_by(|a, b| {
        let a_score = (if a.window_title.is_some() { 4 } else { 0 })
            + (if !a.sockets.is_empty() { 2 } else { 0 })
            + (if a.is_likely_game { 1 } else { 0 });
        let b_score = (if b.window_title.is_some() { 4 } else { 0 })
            + (if !b.sockets.is_empty() { 2 } else { 0 })
            + (if b.is_likely_game { 1 } else { 0 });
        b_score.cmp(&a_score).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    result
}

/// Returns all active sockets for a specific PID.
pub fn get_sockets_for_pid(pid: u32) -> Vec<ProcessSocket> {
    get_all_process_sockets().remove(&pid).unwrap_or_default()
}

/// Generates an optimized WinDivert filter expression targeting the given sockets.
/// If outbound and inbound are both true, intercepts bidirectional traffic for those ports.
pub fn build_filter_for_sockets(
    sockets: &[ProcessSocket],
    include_tcp: bool,
    include_udp: bool,
    inbound: bool,
    outbound: bool,
) -> String {
    let mut tcp_ports: Vec<u16> = if include_tcp {
        sockets
            .iter()
            .filter(|s| s.protocol == SocketProtocol::Tcp && s.local_port > 0)
            .map(|s| s.local_port)
            .collect()
    } else {
        Vec::new()
    };
    tcp_ports.sort_unstable();
    tcp_ports.dedup();

    let mut udp_ports: Vec<u16> = if include_udp {
        sockets
            .iter()
            .filter(|s| s.protocol == SocketProtocol::Udp && s.local_port > 0)
            .map(|s| s.local_port)
            .collect()
    } else {
        Vec::new()
    };
    udp_ports.sort_unstable();
    udp_ports.dedup();

    let dir = match (inbound, outbound) {
        (true, true) => "(inbound or outbound)",
        (true, false) => "inbound",
        (false, true) => "outbound",
        (false, false) => "(inbound or outbound)",
    };

    let mut port_clauses: Vec<String> = Vec::new();

    for p in tcp_ports {
        port_clauses.push(format!("tcp.SrcPort == {p} or tcp.DstPort == {p}"));
    }
    for p in udp_ports {
        port_clauses.push(format!("udp.SrcPort == {p} or udp.DstPort == {p}"));
    }

    if port_clauses.is_empty() {
        return dir.to_string();
    }

    format!("{dir} and ({})", port_clauses.join(" or "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_processes_and_sockets() {
        let procs = list_processes();
        assert!(!procs.is_empty(), "Should list running processes on Windows");
        let has_sockets = procs.iter().any(|p| !p.sockets.is_empty());
        println!("Total processes: {}, Has processes with sockets: {}", procs.len(), has_sockets);
        assert!(has_sockets, "At least one process should have active sockets");
    }

    #[test]
    fn test_filter_generation() {
        let sockets = vec![
            ProcessSocket {
                protocol: SocketProtocol::Udp,
                local_port: 27015,
                remote_port: 0,
                remote_addr: String::new(),
                state: "UDP".into(),
            },
            ProcessSocket {
                protocol: SocketProtocol::Tcp,
                local_port: 64200,
                remote_port: 443,
                remote_addr: "1.2.3.4".into(),
                state: "ESTABLISHED".into(),
            },
        ];
        let filter = build_filter_for_sockets(&sockets, true, true, true, true);
        assert!(filter.contains("udp.SrcPort == 27015 or udp.DstPort == 27015"));
        assert!(filter.contains("tcp.SrcPort == 64200 or tcp.DstPort == 64200"));
        assert!(filter.contains("(inbound or outbound)"));
    }
}
