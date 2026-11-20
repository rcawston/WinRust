//! Settings shared by the WinRust programs: `%APPDATA%\WinRust\config.txt`, one `key = value` per line, `#` starts a comment
//! line. Per user and writable without admin rights, so it can be edited by hand; the bar's right-click menu changes the
//! same file. Programs re-read it at most once a second and at once on the broadcast `set` sends (`msg()`).
//!
//! Every key is listed in `KEYS` with its default: a missing or unreadable value falls back to it.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering::Relaxed},
    },
};
use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, PostMessageW, RegisterWindowMessageW};

/// Every setting: (key, default, what it does). `ensure` writes them all into the file, so it documents itself.
pub const KEYS: &[(&str, &str, &str)] = &[
    ("topbar.autohide", "0", "1 = the bar slides away when the pointer leaves it and comes back at the screen edge (nothing static on an OLED screen)"),
    ("topbar.autohide_delay_ms", "800", "how long the bar stays after the pointer left it, in milliseconds (0..60000)"),
    ("topbar.align", "center", "center or left: where Start and the app buttons sit"),
    ("topbar.labels", "0", "1 = an open window gets a wide button with its title, 0 = icons only"),
    ("topbar.label_width", "160", "width of a button with a title, in pixels at 100% scaling (80..400)"),
    ("topbar.search", "1", "0 = no search button on the bar"),
    ("topbar.gear", "0", "1 = a gear button at the left end of the bar (the same menus are in the bar's right-click menu)"),
    ("desktop.autohide_icons", "0", "1 = the desktop icons stay hidden until the desktop is clicked"),
    ("desktop.icons_delay_s", "5", "seconds without mouse activity on the desktop after which the icons hide again (1..3600)"),
    ("start.buttons", "lock,sleep,restart,shutdown", "buttons at the bottom of Start, any of: lock, signout, sleep, hibernate, restart, shutdown"),
    ("start.power_menu", "0", "1 = one power button that opens a menu with those actions, instead of a row of buttons"),
    ("explorer.tabs", "1", "0 = the file manager has no tabs: one folder per window"),
];

pub fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("WinRust").join("config.txt"))
}

/// The `key = value` lines of `text`. Keys are lower-cased; comment lines (`#`, `;`), blank and malformed lines are skipped.
pub fn parse(text: &str) -> HashMap<String, String> {
    text.trim_start_matches('\u{feff}') // Notepad's UTF-8 mark
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') || l.starts_with(';') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            let k = k.trim().to_ascii_lowercase();
            (!k.is_empty()).then(|| (k, v.trim().to_string()))
        })
        .collect()
}

/// "1", "true", "yes", "on" / "0", "false", "no", "off" in any case; None for anything else.
pub fn truthy(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn sets(line: &str, key: &str) -> bool {
    let l = line.trim();
    !l.starts_with('#') && !l.starts_with(';') && l.split_once('=').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key))
}

/// `text` with `key` set to `value`: the line that sets it is rewritten in place (comments and the other lines stay),
/// or a new line is added at the end.
pub fn with_value(text: &str, key: &str, value: &str) -> String {
    let mut done = false;
    let mut out: Vec<String> = Vec::new();
    for l in text.trim_start_matches('\u{feff}').lines() {
        if !sets(l, key) {
            out.push(l.to_string());
        } else if !done {
            out.push(format!("{key} = {value}")); // a second line for the same key is dropped
            done = true;
        }
    }
    if !done {
        out.push(format!("{key} = {value}"));
    }
    out.join("\r\n") + "\r\n"
}

/// `text` plus every known key it does not set yet, each under a comment that explains it. An empty text gives the
/// whole documented file.
pub fn complete(text: &str) -> String {
    let have = parse(text);
    let mut out = text.trim_start_matches('\u{feff}').lines().map(str::to_string).collect::<Vec<_>>();
    if out.is_empty() {
        out.push("# WinRust settings. Saved changes apply within a second; the bar's right-click menu edits this file too.".to_string());
    }
    for (k, d, help) in KEYS.iter().filter(|k| !have.contains_key(k.0)) {
        out.extend([String::new(), format!("# {help}"), format!("{k} = {d}")]);
    }
    out.join("\r\n") + "\r\n"
}

fn default_of(key: &str) -> &'static str {
    KEYS.iter().find(|k| k.0 == key).map_or("", |k| k.1)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

static CACHE: Mutex<Option<(HashMap<String, String>, u64)>> = Mutex::new(None); // the parsed file and when it was read

fn file_text() -> String {
    path().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default()
}

/// Reads the file again now (on the "settings changed" broadcast).
pub fn reload() {
    *CACHE.lock().unwrap() = Some((parse(&file_text()), now_ms()));
}

fn stored(key: &str) -> Option<String> {
    let mut g = CACHE.lock().unwrap();
    if g.as_ref().is_none_or(|(_, at)| now_ms().saturating_sub(*at) > 1000) {
        *g = Some((parse(&file_text()), now_ms()));
    }
    g.as_ref().and_then(|(m, _)| m.get(key).cloned())
}

/// The value of `key` as text: from the file, else its default.
pub fn text(key: &str) -> String {
    stored(key).unwrap_or_else(|| default_of(key).to_string())
}

/// An on / off setting. Anything that is not a yes or a no counts as the default.
pub fn flag(key: &str) -> bool {
    stored(key).and_then(|v| truthy(&v)).or_else(|| truthy(default_of(key))).unwrap_or(false)
}

/// A number, kept within `min..=max`. Anything that is not a number counts as the default.
pub fn int(key: &str, min: i64, max: i64) -> i64 {
    stored(key).and_then(|v| v.trim().parse().ok()).or_else(|| default_of(key).parse().ok()).unwrap_or(min).clamp(min, max)
}

/// Writes one setting (the rest of the file stays as it is), takes it in this process at once and tells the other
/// WinRust windows to read the file again.
pub fn set(key: &str, value: &str) {
    let Some(f) = path() else { return };
    let old = std::fs::read_to_string(&f).unwrap_or_else(|_| complete(""));
    let new = with_value(&old, key, value);
    let _ = std::fs::create_dir_all(f.parent().unwrap());
    let _ = std::fs::write(&f, &new);
    *CACHE.lock().unwrap() = Some((parse(&new), now_ms()));
    unsafe { PostMessageW(HWND_BROADCAST, msg(), 0, 0) };
}

/// Makes sure the file exists and lists every known setting (for "open the settings file"). Returns its path.
pub fn ensure() -> Option<PathBuf> {
    let f = path()?;
    let old = std::fs::read_to_string(&f).unwrap_or_default();
    let new = complete(&old);
    if new != old {
        std::fs::create_dir_all(f.parent()?).ok()?;
        std::fs::write(&f, new).ok()?;
    }
    Some(f)
}

/// The broadcast message a window gets when a setting changed: call `reload()` and apply.
pub fn msg() -> u32 {
    static M: AtomicU32 = AtomicU32::new(0);
    match M.load(Relaxed) {
        0 => {
            let n: Vec<u16> = "WinRustConfigChanged\0".encode_utf16().collect();
            let m = unsafe { RegisterWindowMessageW(n.as_ptr()) };
            M.store(m, Relaxed);
            m
        }
        m => m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_and_skips_comments() {
        let m = parse("\u{feff}# a comment\r\nTopbar.Autohide = 1\r\n\r\n; another\r\ntopbar.align=left \r\nno equals sign\r\n = 3\r\n");
        assert_eq!(m.len(), 2);
        assert_eq!(m["topbar.autohide"], "1");
        assert_eq!(m["topbar.align"], "left");
    }

    #[test]
    fn yes_and_no_words() {
        assert_eq!((truthy(" On "), truthy("0"), truthy("FALSE"), truthy("maybe")), (Some(true), Some(false), Some(false), None));
    }

    #[test]
    fn a_value_is_rewritten_in_place() {
        let t = "# head\r\ntopbar.search = 1\r\n# tail\r\ntopbar.search = 0\r\n";
        assert_eq!(with_value(t, "topbar.search", "0"), "# head\r\ntopbar.search = 0\r\n# tail\r\n");
        assert_eq!(with_value("a = 1\n", "b", "2"), "a = 1\r\nb = 2\r\n");
        assert_eq!(with_value("# b = 5\n", "b", "2"), "# b = 5\r\nb = 2\r\n"); // a commented-out line is not the setting
    }

    #[test]
    fn the_file_lists_every_key_once() {
        let full = complete("");
        let m = parse(&full);
        assert_eq!(m.len(), KEYS.len());
        assert!(KEYS.iter().all(|k| m[k.0] == k.1));
        assert_eq!(complete(&full), full); // nothing to add the second time
        let mine = complete("topbar.autohide = 1\r\n");
        assert_eq!(parse(&mine)["topbar.autohide"], "1"); // the user's value stays
        assert_eq!(parse(&mine).len(), KEYS.len());
    }

    #[test]
    fn defaults_are_valid() {
        assert_eq!(default_of("topbar.align"), "center");
        assert_eq!(default_of("no.such.key"), "");
        assert!(KEYS.iter().all(|k| k.0 == k.0.to_ascii_lowercase() && !k.1.is_empty()));
    }
}
