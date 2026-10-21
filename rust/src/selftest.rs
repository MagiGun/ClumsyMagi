//! `--selftest` runtime checks: verifies the WinDivert DLL loads, that filter
//! compilation works (no admin needed), and — when elevated — that a handle
//! can actually be opened. Used by CI and first-run diagnostics.

use crate::wd::{WinDivertLib, WINDIVERT_PARAM_QUEUE_LENGTH};

pub fn run() -> bool {
    println!("ClumsyMagi {} selftest", crate::app::VERSION);
    println!("platform: windows, elevated: {}", crate::platform::is_elevated());
    let mut ok = true;

    let lib = match WinDivertLib::load() {
        Ok(l) => {
            println!("[PASS] WinDivert.dll loaded from {}", l.path.display());
            l
        }
        Err(e) => {
            println!("[FAIL] WinDivert.dll: {}", e.friendly());
            return false;
        }
    };

    // filter compilation
    let cases: [(&str, bool); 4] = [
        ("true", true),
        ("tcp and outbound", true),
        ("outbound and ip.DstAddr >= 127.0.0.1 and ip.DstAddr <= 127.255.255.255", true),
        ("tcp and", false),
    ];
    for (f, should_pass) in cases {
        match lib.compile_filter(f) {
            Ok(()) if should_pass => println!("[PASS] filter compiles: {f}"),
            Ok(()) => {
                println!("[FAIL] filter should have been rejected: {f}");
                ok = false;
            }
            Err(e) if !should_pass => println!("[PASS] bad filter rejected: {f} -> {e}"),
            Err(e) => {
                println!("[FAIL] filter should have compiled: {f} -> {e}");
                ok = false;
            }
        }
    }

    // opening a real handle (requires admin; informational otherwise)
    match lib.open("true") {
        Ok(handle) => {
            println!("[PASS] WinDivertOpen succeeded (elevated)");
            lib.set_param_on(handle, WINDIVERT_PARAM_QUEUE_LENGTH, 2048);
            lib.close_on(handle);
        }
        Err(e) => {
            if crate::platform::is_elevated() {
                println!("[FAIL] WinDivertOpen failed while elevated: {}", e.friendly());
                ok = false;
            } else {
                println!("[WARN] WinDivertOpen failed (expected without admin): {}", e.friendly());
            }
        }
    }

    // WdAddress raw roundtrip
    let mut addr = crate::wd::WdAddress::default();
    addr.outbound = true;
    addr.loopback = true;
    addr.ipv6 = true;
    addr.layer = 0;
    addr.event = 0;
    addr.data[0] = 0x42;
    let mut raw = [0u8; crate::wd::WINDIVERT_ADDRESS_SIZE];
    addr.to_raw(&mut raw);
    let back = crate::wd::WdAddress::from_raw(&raw);
    if back.outbound && back.loopback && back.ipv6 && back.data[0] == 0x42 {
        println!("[PASS] WINDIVERT_ADDRESS encode/decode roundtrip");
    } else {
        println!("[FAIL] WINDIVERT_ADDRESS roundtrip mismatch: {back:?}");
        ok = false;
    }

    println!("selftest {}", if ok { "PASSED" } else { "FAILED" });
    ok
}
