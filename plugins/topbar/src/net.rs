// Wired network adapters and the Bluetooth radio, for the bar's network icon, the quick panel and the network panel.
use std::{
    ptr::null,
    sync::{
        Mutex,
        atomic::{AtomicI32, Ordering::Relaxed},
    },
};
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows_sys::Win32::{
    Devices::Bluetooth::{BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS, BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindNextDevice},
    Foundation::HWND,
    Graphics::Gdi::InvalidateRect,
    NetworkManagement::IpHelper::{GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH},
};

/// A wired adapter with a live link.
pub struct Eth {
    pub name: String, // "Ethernet", as Windows calls the connection
    pub desc: String, // the adapter's model
    pub bps: u64,     // link speed, 0 = unknown
    pub ip: String,   // IPv4 address, "" = none yet
}

/// Software adapters that report a link all the time (virtual switches, VPN drivers): they are not "the cable".
pub fn is_virtual(desc: &str) -> bool {
    let d = desc.to_lowercase();
    ["virtual", "hyper-v", "vmware", "loopback", "tap-", "wintun", "wireguard", "vpn", "bluetooth", "npcap", "tunnel"].iter().any(|v| d.contains(v))
}

/// "1 Gbps", "2.5 Gbps", "100 Mbps"; "" when the driver reports no speed.
pub fn speed_text(bps: u64) -> String {
    match bps {
        0 | u64::MAX => String::new(),
        b if b >= 1_000_000_000 => {
            let tenths = b / 100_000_000;
            if tenths % 10 == 0 { format!("{} Gbps", tenths / 10) } else { format!("{}.{} Gbps", tenths / 10, tenths % 10) }
        }
        b => format!("{} Mbps", b / 1_000_000),
    }
}

unsafe fn pwstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let n = (0..).take_while(|&i| *p.add(i) != 0).count();
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

/// Ethernet adapters whose link is up, the ones with a gateway (a way out) first. An adapter without a gateway counts
/// only when it is real hardware: virtual switches are always "up".
pub unsafe fn ethernets() -> Vec<Eth> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER | GAA_FLAG_INCLUDE_GATEWAYS;
    let mut size = 16 * 1024u32;
    let mut out: Vec<(bool, Eth)> = Vec::new();
    for _ in 0..3 {
        let mut buf = vec![0u64; size as usize / 8 + 1]; // u64: aligned for the structures
        match GetAdaptersAddresses(0, flags, null(), buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH, &mut size) {
            111 => continue, // ERROR_BUFFER_OVERFLOW: `size` now holds what is needed
            0 => {}
            _ => break,
        }
        let mut a = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !a.is_null() {
            let ad = &*a;
            a = ad.Next;
            if ad.IfType != 6 || ad.OperStatus != 1 {
                continue; // not IF_TYPE_ETHERNET_CSMACD, or not IfOperStatusUp
            }
            let (desc, gw) = (pwstr(ad.Description), !ad.FirstGatewayAddress.is_null());
            if !gw && is_virtual(&desc) {
                continue;
            }
            let mut ip = String::new();
            let mut u = ad.FirstUnicastAddress;
            while !u.is_null() && ip.is_empty() {
                let sa = (*u).Address.lpSockaddr as *const u8;
                if !sa.is_null() && *(sa as *const u16) == 2 {
                    // AF_INET: family, port, then the four address bytes
                    let b = std::slice::from_raw_parts(sa.add(4), 4);
                    ip = format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]);
                }
                u = (*u).Next;
            }
            out.push((gw, Eth { name: pwstr(ad.FriendlyName), desc, bps: ad.TransmitLinkSpeed, ip }));
        }
        break;
    }
    out.sort_by_key(|e| !e.0);
    out.into_iter().map(|e| e.1).collect()
}

// ---------- Bluetooth ----------

pub static BT: AtomicI32 = AtomicI32::new(-1); // -1 = no radio (or not asked yet), 0 = off, 1 = on
pub static BT_DEVS: Mutex<Vec<String>> = Mutex::new(Vec::new()); // names of the connected devices

fn bt_radio() -> Option<Radio> {
    Radio::GetRadiosAsync().ok()?.join().ok()?.into_iter().find(|r| r.Kind().ok() == Some(RadioKind::Bluetooth))
}

/// Names of the Bluetooth devices connected right now (classic ones: that is what the Win32 API lists).
fn bt_connected() -> Vec<String> {
    let mut out = vec![];
    unsafe {
        let mut sp: BLUETOOTH_DEVICE_SEARCH_PARAMS = std::mem::zeroed();
        sp.dwSize = size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32;
        sp.fReturnConnected = 1;
        let mut di: BLUETOOTH_DEVICE_INFO = std::mem::zeroed();
        di.dwSize = size_of::<BLUETOOTH_DEVICE_INFO>() as u32;
        let f = BluetoothFindFirstDevice(&sp, &mut di);
        if f.is_null() {
            return out;
        }
        loop {
            let n = di.szName.iter().position(|&c| c == 0).unwrap_or(di.szName.len());
            if di.fConnected != 0 && n > 0 {
                out.push(String::from_utf16_lossy(&di.szName[..n]));
            }
            if BluetoothFindNextDevice(f, &mut di) == 0 {
                break;
            }
        }
        BluetoothFindDeviceClose(f);
    }
    out
}

fn bt_refresh(hw: isize) {
    let state = match bt_radio().and_then(|r| r.State().ok()) {
        Some(RadioState::On) => 1,
        Some(_) => 0,
        None => -1,
    };
    BT.store(state, Relaxed);
    *BT_DEVS.lock().unwrap() = if state == 1 { bt_connected() } else { vec![] };
    unsafe { InvalidateRect(hw as HWND, null(), 0) };
}

/// Reads the radio's state and the connected devices on a thread (the radio API waits for a system service), then
/// repaints the window `hw`.
pub fn bt_read(hw: isize) {
    std::thread::spawn(move || bt_refresh(hw));
}

/// Turns the Bluetooth radio on or off, like the switch in Settings.
pub fn bt_set(on: bool, hw: isize) {
    BT.store(on as i32, Relaxed); // the tile answers at once; bt_refresh corrects it if Windows refused
    std::thread::spawn(move || {
        if let Some(r) = bt_radio() {
            let _ = Radio::RequestAccessAsync().and_then(|a| a.join());
            let _ = r.SetStateAsync(if on { RadioState::On } else { RadioState::Off }).and_then(|a| a.join());
        }
        bt_refresh(hw);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_speed_reads_like_windows() {
        assert_eq!(speed_text(1_000_000_000), "1 Gbps");
        assert_eq!(speed_text(2_500_000_000), "2.5 Gbps");
        assert_eq!(speed_text(100_000_000), "100 Mbps");
        assert_eq!(speed_text(0), "");
        assert_eq!(speed_text(u64::MAX), "");
    }

    #[test]
    fn virtual_switches_are_not_the_cable() {
        assert!(is_virtual("Hyper-V Virtual Ethernet Adapter"));
        assert!(is_virtual("TAP-Windows Adapter V9"));
        assert!(!is_virtual("Intel(R) Ethernet Controller I225-V"));
        assert!(!is_virtual("Realtek PCIe GbE Family Controller"));
    }

    #[test]
    fn adapters_can_be_listed() {
        // real system check: whatever the machine has, the call must not fail and names are never empty
        assert!(unsafe { ethernets() }.iter().all(|e| !e.name.is_empty()));
    }
}
