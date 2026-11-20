// Desktop icons that stay out of sight until the desktop is clicked, and leave again after a while without mouse
// activity there (`desktop.autohide_icons`, `desktop.icons_delay_s`). Works on Explorer's desktop and on rust-explorer's:
// both keep the icons in a SysListView32, which is simply hidden and shown.
use super::*;

const PROP: &str = "WinRustIconsHidden"; // set on the list while it is hidden by us: any later instance (or --restore) can undo it

thread_local! {
    static LAST: Cell<u64> = const { Cell::new(0) }; // last mouse activity on the desktop
    static PT: Cell<(i32, i32)> = const { Cell::new((0, 0)) }; // where the pointer was at the last tick
}

unsafe fn class_of(h: HWND) -> String {
    let mut c = [0u16; 32];
    let n = GetClassNameW(h, c.as_mut_ptr(), 32).max(0) as usize;
    String::from_utf16_lossy(&c[..n])
}

/// The desktop's icon list. Explorer: Progman (or, with a slideshow / live wallpaper, a WorkerW) > SHELLDLL_DefView >
/// SysListView32. rust-explorer: Progman > SysListView32, next to an empty SHELLDLL_DefView.
unsafe fn list() -> HWND {
    let (dv, lv) = (wide("SHELLDLL_DefView"), wide("SysListView32"));
    for cls in ["Progman", "WorkerW"] {
        let (c, mut top) = (wide(cls), null_mut());
        loop {
            top = FindWindowExW(null_mut(), top, c.as_ptr(), null());
            if top.is_null() {
                break;
            }
            let view = FindWindowExW(top, null_mut(), dv.as_ptr(), null());
            for parent in [view, top] {
                let l = if parent.is_null() { null_mut() } else { FindWindowExW(parent, null_mut(), lv.as_ptr(), null()) };
                if !l.is_null() {
                    return l;
                }
            }
        }
    }
    null_mut()
}

/// The point is on the desktop itself, not on a window above it.
unsafe fn on_desktop(pt: POINT) -> bool {
    matches!(class_of(GetAncestor(WindowFromPoint(pt), GA_ROOT)).as_str(), "Progman" | "WorkerW")
}

/// The desktop is in the middle of something the icons must not vanish from: a menu, a drag, a rename.
unsafe fn busy(lv: HWND) -> bool {
    let mut gi: GUITHREADINFO = zeroed();
    gi.cbSize = size_of::<GUITHREADINFO>() as u32;
    if GetGUIThreadInfo(GetWindowThreadProcessId(lv, null_mut()), &mut gi) == 0 {
        return false;
    }
    gi.flags & (GUI_INMENUMODE | GUI_INMOVESIZE | GUI_POPUPMENUMODE | GUI_SYSTEMMENUMODE) != 0 || !gi.hwndCapture.is_null() || (!gi.hwndFocus.is_null() && class_of(gi.hwndFocus) == "Edit")
}

unsafe fn show(lv: HWND, on: bool) {
    let p = wide(PROP);
    if on {
        RemovePropW(lv, p.as_ptr());
        ShowWindow(lv, SW_SHOWNA);
    } else {
        SetPropW(lv, p.as_ptr(), 1 as HANDLE);
        ShowWindow(lv, SW_HIDE);
    }
}

unsafe fn shown(lv: HWND) -> bool {
    GetWindowLongW(lv, GWL_STYLE) as u32 & WS_VISIBLE != 0
}

/// A mouse button went down at `pt` (from the mouse hook): on the desktop it brings the icons back.
pub unsafe fn clicked(pt: POINT) {
    if !cfg().desk_icons || !on_desktop(pt) {
        return;
    }
    LAST.set(now_ms());
    let lv = list();
    if !lv.is_null() && !shown(lv) {
        show(lv, true);
    }
}

/// Four times a second: hides the icons once the desktop has been left alone for the configured time.
pub unsafe fn tick() {
    let c = cfg();
    if !c.desk_icons {
        return;
    }
    let lv = list();
    if lv.is_null() || !shown(lv) {
        return;
    }
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    let moved = PT.replace((pt.x, pt.y)) != (pt.x, pt.y);
    let now = now_ms();
    if (moved && on_desktop(pt)) || busy(lv) {
        LAST.set(now);
    }
    if now.saturating_sub(LAST.get()) >= c.desk_s * 1000 {
        show(lv, false);
    }
}

/// Brings back icons that were hidden by us (the setting was turned off, the bar is closing, `--restore`).
pub unsafe fn restore() {
    let lv = list();
    if !lv.is_null() && !GetPropW(lv, wide(PROP).as_ptr()).is_null() {
        show(lv, true);
    }
}
