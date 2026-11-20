// Own flyouts instead of the stock Windows ones: Start (search + all apps + power), the quick panel (network, Bluetooth,
// volume, brightness, battery), the network panel (Ethernet + Wi-Fi), the sound panel (devices, microphone), calendar.
// One popup window; what it shows depends on `Kind`.
use super::*;
use std::sync::Mutex;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicI32, Ordering::Relaxed},
};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Quick,
    Wifi,  // the network panel: wired connections, then the Wi-Fi list
    Sound, // outputs, inputs and the microphone
    Cal,
    Start,
    Menu, // right-click menu of the bar
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    VolSlide,
    BrightSlide,
    Mute,
    SoundMore, // the arrow next to the volume slider: the sound panel
    OutDev(usize),
    InDev(usize),
    MicSlide,
    MicMute,
    Mixer,
    WifiSwitch,
    Tile(u8),
    WifiMore,
    BtMore, // the Bluetooth tile's arrow: its settings page
    Net(usize),
    NetOff, // the connected network's Disconnect button
    Settings(&'static str),
    Menu(u8),
    CalPrev,
    CalNext,
    CalToday,
    App(usize), // index into the filtered list
    Power(u8),  // an index into POWER, or POWER_MENU: the one button that opens them as a menu
}

/// Start's power actions in the order they are shown; these names are what `start.buttons` in config.txt lists.
pub const POWER: [&str; 6] = ["lock", "signout", "sleep", "hibernate", "restart", "shutdown"];
const POWER_GLYPH: [&str; 7] = ["\u{E72E}", "\u{E77B}", "\u{E708}", "\u{E823}", "\u{E777}", "\u{E7E8}", "\u{E7E8}"];
const POWER_MENU: u8 = 6;

/// "lock, Restart" -> the bits of those actions (unknown names are ignored).
pub fn power_mask(list: &str) -> u8 {
    list.split(',').filter_map(|n| POWER.iter().position(|p| p.eq_ignore_ascii_case(n.trim()))).fold(0, |m, i| m | 1 << i)
}

/// The bits back as the list config.txt holds.
pub fn power_names(mask: u8) -> String {
    (0..POWER.len()).filter(|i| mask & 1 << i != 0).map(|i| POWER[i]).collect::<Vec<_>>().join(",")
}

pub fn power_label(p: u8) -> &'static str {
    match p {
        0 => t!("Блокировка", "Lock"),
        1 => t!("Выход из системы", "Sign out"),
        2 => t!("Сон", "Sleep"),
        3 => t!("Гибернация", "Hibernate"),
        4 => t!("Перезагрузка", "Restart"),
        5 => t!("Завершение работы", "Shut down"),
        _ => t!("Питание", "Power"),
    }
}

/// The buttons at the bottom of Start, left to right: the chosen actions, or the one menu button.
fn power_shown() -> Vec<u8> {
    let c = cfg();
    match (c.power, c.power_menu) {
        (0, _) => vec![],
        (_, true) => vec![POWER_MENU],
        (m, false) => (0..POWER.len() as u8).filter(|i| m & 1 << i != 0).collect(),
    }
}

/// Signing out, restarting and shutting down close the user's work: as buttons they take a second click.
fn power_confirm(p: u8) -> bool {
    matches!(p, 1 | 4 | 5)
}

unsafe fn power(p: u8) {
    let run = |exe: &str, args: &[&str]| _ = Command::new(exe).args(args).creation_flags(NO_WINDOW).spawn();
    match p {
        0 => run("rundll32.exe", &["user32.dll,LockWorkStation"]),
        1 => run("shutdown", &["/l"]),
        // not `rundll32 powrprof.dll,SetSuspendState`: rundll32 passes garbage arguments and the PC hibernates
        // instead of sleeping whenever hibernation is on. On its own thread: the call returns only after wake-up.
        2 => _ = std::thread::spawn(sleep_now),
        3 => run("shutdown", &["/h"]),
        4 => run("shutdown", &["/r", "/t", "0"]),
        _ => run("shutdown", &["/s", "/t", "0"]),
    }
}

/// The power actions as a menu (Start's single power button), centered on screen x `cx`.
unsafe fn power_menu(cx: i32) {
    let ids: Vec<u8> = (0..POWER.len() as u8).filter(|i| cfg().power & 1 << i != 0).collect();
    let items = ids.iter().map(|&p| if power_confirm(p) { MItem::new(power_label(p)).danger() } else { MItem::new(power_label(p)) }).collect();
    menu(cx, None, items, move |i| {
        if let Some(&p) = ids.get(i) {
            power(p);
        }
    });
}

struct Net {
    ssid: String,
    q: i32,
    secure: bool,
    profile: Option<String>,
    connected: bool,
    auth: i32,
}

struct Conn {
    ssid: String,
    auth: i32,
    q: i32,
    rx: u32, // Mbps
    tx: u32,
    phy: &'static str,
}

fn auth_name(a: i32) -> &'static str {
    match a {
        1 => "Open",
        2 => "WEP",
        3 => "WPA",
        4 => "WPA‑Personal",
        6 => "WPA2",
        7 => "WPA2‑Personal",
        8 => "WPA3",
        9 => "WPA3‑Personal",
        _ => "Secured",
    }
}

fn phy_name(t: i32) -> &'static str {
    match t {
        7 => "Wi‑Fi 4",
        8 => "Wi‑Fi 5",
        10 => "Wi‑Fi 6",
        11 => "Wi‑Fi 7",
        4..=6 => "802.11a/b/g",
        _ => "Wi‑Fi",
    }
}

static APPS: Mutex<Vec<(String, Vec<u16>)>> = Mutex::new(Vec::new()); // Start menu shortcuts: (name, .lnk path)
static BRIGHT: AtomicI32 = AtomicI32::new(-1); // -1 = no controllable display (desktop PC)
const ROWS: usize = 11;

thread_local! {
    static FLY: Cell<Option<Kind>> = const { Cell::new(None) };
    static FH: Cell<HWND> = const { Cell::new(null_mut()) };
    static CAL: Cell<(i32, i32)> = const { Cell::new((2026, 1)) };
    static NETS: RefCell<Vec<Net>> = const { RefCell::new(Vec::new()) };
    static CONN: RefCell<Option<Conn>> = const { RefCell::new(None) };
    static PASS: RefCell<Option<(usize, String)>> = const { RefCell::new(None) };
    static NOTE: RefCell<String> = const { RefCell::new(String::new()) };
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static SCROLL: Cell<usize> = const { Cell::new(0) };
    static ICONS: RefCell<HashMap<Vec<u16>, HICON>> = RefCell::new(HashMap::new()); // by path: the app list is reloaded and reordered
    static SEL: Cell<usize> = const { Cell::new(0) }; // Start: the row Enter opens (arrow keys move it)
    static OVER: Cell<Option<Act>> = const { Cell::new(None) };
    static DRAG: Cell<u8> = const { Cell::new(0) }; // 0 none, 1 volume slider, 2 brightness slider
    static MENU_X: Cell<i32> = const { Cell::new(0) };
    static CONFIRM: Cell<i32> = const { Cell::new(-1) };
    static LAST: Cell<(Option<Kind>, u64)> = const { Cell::new((None, 0)) }; // which flyout closed when
    static FIN: Cell<(i32, i32)> = const { Cell::new((0, 0)) }; // where the open flyout rests
    static T0: Cell<u64> = const { Cell::new(0) }; // when it started opening
    static JUST: RefCell<(String, u64)> = const { RefCell::new((String::new(), 0)) }; // the app pinned last and when: its row pulses
    static GHOST: Cell<(HWND, u64, (i32, i32))> = const { Cell::new((null_mut(), 0, (0, 0))) }; // the closing flyout: window, start, position
    static OUTS: RefCell<Vec<audio::Dev>> = const { RefCell::new(Vec::new()) }; // sound panel: outputs
    static INS: RefCell<Vec<audio::Dev>> = const { RefCell::new(Vec::new()) }; // ...inputs
    static MIC: Cell<(i32, bool)> = const { Cell::new((-1, false)) }; // ...the default microphone: (level or -1 = none, muted)
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

// ---------- data ----------

/// Background: collects the Start menu shortcuts once.
pub fn load_apps() {
    fn walk(dir: &std::path::Path, depth: u8, out: &mut Vec<(String, Vec<u16>)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 4 {
                walk(&p, depth + 1, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
                let name = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
                if !name.to_lowercase().contains("uninstall") {
                    out.push((name, wide(&p.to_string_lossy())));
                }
            }
        }
    }
    let mut v = Vec::new();
    for (var, sub) in [("ProgramData", r"Microsoft\Windows\Start Menu\Programs"), ("APPDATA", r"Microsoft\Windows\Start Menu\Programs")] {
        if let Some(b) = std::env::var_os(var) {
            walk(&PathBuf::from(b).join(sub), 0, &mut v);
        }
    }
    // Store apps (Settings, Calculator, Terminal, Photos...) have no .lnk: the shell's Apps folder lists them
    let have: std::collections::HashSet<String> = v.iter().map(|a| a.0.to_lowercase()).collect();
    for (name, id) in apps_folder() {
        let ok = id.contains('!') || id.to_lowercase().ends_with(".exe") || id.starts_with("Microsoft.Windows."); // apps, not docs / web links
        if ok && !have.contains(&name.to_lowercase()) && !name.to_lowercase().contains("uninstall") {
            v.push((name, wide(&format!(r"shell:AppsFolder\{id}"))));
        }
    }
    v.sort_by_key(|a| a.0.to_lowercase());
    v.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
    *APPS.lock().unwrap() = v;
}

/// (display name, app id) of everything in the shell's Apps folder (what the Windows Start menu lists).
fn apps_folder() -> Vec<(String, String)> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING},
    };
    let mut out = vec![];
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // this runs on its own thread
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return out };
        let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut items = [None];
            let mut got = 0u32;
            if en.Next(&mut items, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(it) = items[0].take() else { break };
            let name = |k: SIGDN| {
                it.GetDisplayName(k).ok().map(|p| {
                    let s = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const _));
                    s
                })
            };
            if let (Some(n), Some(id)) = (name(SIGDN_NORMALDISPLAY), name(SIGDN_PARENTRELATIVEPARSING)) {
                out.push((n, id));
            }
        }
    }
    out
}

const EN_KEYS: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`";
const RU_KEYS: &str = "йцукенгшщзхъфывапролджэячсмитьбюё";

/// The same keys typed in the other layout: "сркщьу" -> "chrome", "ntktuhfv" -> "телеграм".
fn flip_layout(q: &str) -> String {
    let (en, ru): (Vec<char>, Vec<char>) = (EN_KEYS.chars().collect(), RU_KEYS.chars().collect());
    q.chars().map(|c| en.iter().position(|&x| x == c).map(|i| ru[i]).or_else(|| ru.iter().position(|&x| x == c).map(|i| en[i])).unwrap_or(c)).collect()
}

/// How well an app name matches the query (both lower-case): 0 the name starts with it, 1 a word does, 2 the word
/// initials do ("vsc" = Visual Studio Code), 3 it is somewhere inside; None = no match.
fn score(name: &str, q: &str) -> Option<u8> {
    if q.is_empty() {
        return None;
    }
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
    if name.starts_with(q) {
        Some(0)
    } else if words.iter().any(|w| w.starts_with(q)) {
        Some(1)
    } else if q.chars().count() >= 2 && initials.starts_with(q) {
        Some(2)
    } else if name.contains(q) {
        Some(3)
    } else {
        None
    }
}

// ---------- apps pinned to the Start panel (empty at first; added from right-click menus) ----------

static PINNED: Mutex<Vec<String>> = Mutex::new(Vec::new()); // paths of .lnk / .exe files, in the order they were added

fn pin_file() -> PathBuf {
    appdata_file("topbar-start.txt")
}

fn pinned_load() {
    *PINNED.lock().unwrap() = std::fs::read_to_string(pin_file()).map(|s| s.lines().filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default();
}

fn wstr(w: &[u16]) -> String {
    String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
}

pub fn start_pinned(path: &str) -> bool {
    PINNED.lock().unwrap().iter().any(|p| p.eq_ignore_ascii_case(path))
}

/// Files dropped on the open Start panel are added to it; the panel stays open and the new row pulses.
unsafe fn on_files(hw: HWND, hd: HDROP) {
    let n = DragQueryFileW(hd, u32::MAX, null_mut(), 0);
    let mut added = vec![];
    for i in 0..n {
        let len = DragQueryFileW(hd, i, null_mut(), 0) as usize;
        let mut b = vec![0u16; len + 1];
        DragQueryFileW(hd, i, b.as_mut_ptr(), b.len() as u32);
        let path = String::from_utf16_lossy(&b[..len]);
        if start_pin(&path) {
            added.push(std::path::Path::new(&path).file_stem().unwrap_or_default().to_string_lossy().to_string());
        }
    }
    DragFinish(hd);
    note(&match added.len() {
        0 => "Already in Start".to_string(),
        1 => format!("Added to Start: {}", added[0]),
        n => format!("Added {n} items to Start"),
    });
    KillTimer(hw, 6);
    InvalidateRect(hw, null(), 0);
    force_fg(hw); // active again, so a later click elsewhere closes it as usual
}

/// Adds the app to the Start panel unless it is already there (a drop on the Start button). true = it was added.
pub fn start_pin(path: &str) -> bool {
    if start_pinned(path) {
        return false;
    }
    start_toggle(path);
    JUST.set((path.to_string(), unsafe { now_ms() }));
    true
}

pub fn is_open(k: Kind) -> bool {
    FLY.get() == Some(k)
}

pub fn any_open() -> bool {
    FLY.get().is_some()
}

/// Short status line shown at the bottom of the open panel (Start: "Added to Start: ...").
pub fn note(s: &str) {
    NOTE.set(s.to_string());
}

/// Adds the app to the Start panel, or removes it when it is already there.
pub fn start_toggle(path: &str) {
    let mut p = PINNED.lock().unwrap();
    match p.iter().position(|x| x.eq_ignore_ascii_case(path)) {
        Some(i) => drop(p.remove(i)),
        None => p.push(path.to_string()),
    }
    let _ = std::fs::write(pin_file(), p.join("\n"));
}

/// What a Start layout exported by Windows pins, in its order: shortcut paths (environment variables as written) and
/// `shell:AppsFolder\<id>` for apps named by id. Reads both formats of `Export-StartLayout`: the XML of Windows 10
/// (attributes) and the JSON of Windows 11 (keys).
fn layout_entries(text: &str) -> Vec<String> {
    const KEYS: [(&str, bool); 6] = [("DesktopApplicationLinkPath=\"", false), ("DesktopApplicationID=\"", true), ("AppUserModelID=\"", true), ("\"desktopAppLink\":", false), ("\"packagedAppId\":", true), ("\"desktopAppId\":", true)];
    let mut found: Vec<(usize, String)> = vec![];
    for (key, is_id) in KEYS {
        let json = key.ends_with(':');
        let mut at = 0;
        while let Some(i) = text[at..].find(key) {
            let mut v = &text[at + i + key.len()..];
            at += i + key.len();
            if json {
                let Some(q) = v.trim_start().strip_prefix('"') else { continue };
                v = q;
            }
            let Some(end) = v.find('"') else { break };
            let val = if json { v[..end].replace("\\\\", "\\") } else { v[..end].replace("&amp;", "&") };
            if !val.is_empty() {
                found.push((at, if is_id { format!(r"shell:AppsFolder\{val}") } else { val }));
            }
        }
    }
    found.sort_by_key(|f| f.0);
    found.into_iter().map(|f| f.1).collect()
}

/// What the user has pinned in Windows: the Start menu's tiles (as `Export-StartLayout` lists them) and the Quick Launch
/// shortcuts. Only what can be started from here: shortcuts that exist and apps the Start list knows.
fn windows_pinned() -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let tmp = std::env::temp_dir().join("winrust-start-layout.xml");
    let _ = std::fs::remove_file(&tmp);
    let _ = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", "Export-StartLayout -Path $env:WR_OUT"]).env("WR_OUT", &tmp).creation_flags(NO_WINDOW).output();
    if let Ok(b) = std::fs::read(&tmp) {
        let text = if b.starts_with(&[0xFF, 0xFE]) { String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()) } else { String::from_utf8_lossy(&b).to_string() };
        out.extend(layout_entries(&text).iter().map(|e| expand_env(e)));
    }
    let _ = std::fs::remove_file(&tmp);
    for dir in [r"Microsoft\Internet Explorer\Quick Launch", r"Microsoft\Internet Explorer\Quick Launch\User Pinned\StartMenu"] {
        let mut links: Vec<_> = std::fs::read_dir(appdata_file(dir)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk"))).collect();
        links.sort();
        // Windows' own two (show the desktop, switch windows) are not programs
        out.extend(links.iter().map(|p| p.to_string_lossy().to_string()).filter(|p| !p.ends_with("Shows Desktop.lnk") && !p.ends_with("Window Switcher.lnk")));
    }
    let apps = APPS.lock().unwrap();
    out.into_iter()
        .filter_map(|e| match apps.iter().find(|a| wstr(&a.1).eq_ignore_ascii_case(&e)) {
            Some(a) => Some(wstr(&a.1)), // spelled as the Start list has it
            None => (!e.starts_with("shell:") && std::path::Path::new(&e).is_file()).then_some(e),
        })
        .collect()
}

/// Adds what Windows has pinned to our Start (what is there stays, nothing twice). On a thread: PowerShell takes a
/// second; the bar gets IMPORT_DONE with the number added and shows Start.
pub fn start_import() {
    let bar = BAR.load(Relaxed);
    std::thread::spawn(move || {
        if APPS.lock().unwrap().is_empty() {
            load_apps();
        }
        let found = windows_pinned();
        pinned_load();
        let mut p = PINNED.lock().unwrap();
        let before = p.len();
        for f in found {
            if !p.iter().any(|x| x.eq_ignore_ascii_case(&f)) {
                p.push(f);
            }
        }
        let _ = std::fs::write(pin_file(), p.join("\n"));
        unsafe { PostMessageW(bar as HWND, IMPORT_DONE, p.len() - before, 0) };
    });
}

/// The import finished (on the bar's thread): Start opens and says what it got.
pub unsafe fn import_done(added: usize) {
    if !is_open(Kind::Start) {
        toggle(Kind::Start);
    }
    note(&match added {
        0 => t!("В Windows не закреплено ничего нового", "Nothing new is pinned in Windows").to_string(),
        n => tf!("Добавлено из Windows: {}", "Added from Windows: {}", n),
    });
    InvalidateRect(FH.get(), null(), 0);
}

static EXE_GEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Programs that have no Start menu shortcut (a loose loliland.exe): asks rust-search for .exe files matching what is typed
/// and adds them to the app list, where the normal ranking picks them up. Debounced; the panel repaints when they arrive.
fn find_loose_exes(hw: HWND) {
    let q = QUERY.with_borrow(|q| q.trim().to_string());
    let gen_ = EXE_GEN.fetch_add(1, Relaxed) + 1;
    let rs = PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_default()).join(r"rs-search\rs.exe");
    if q.chars().count() < 2 || !rs.is_file() {
        return;
    }
    let hw = hw as isize;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if EXE_GEN.load(Relaxed) != gen_ {
            return; // typed on: a newer search is coming
        }
        let Ok(o) = Command::new(&rs).arg(&q).creation_flags(NO_WINDOW).output() else { return };
        let mut apps = APPS.lock().unwrap();
        let mut added = false;
        for line in String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.to_lowercase().ends_with(".exe")).take(12) {
            let w = wide(line);
            if apps.iter().any(|a| a.1 == w) {
                continue;
            }
            let name = std::path::Path::new(line).file_stem().unwrap_or_default().to_string_lossy().to_string();
            apps.push((name, w));
            added = true;
        }
        drop(apps);
        if added {
            unsafe { InvalidateRect(hw as HWND, null(), 0) };
        }
    });
}

fn filtered() -> Vec<usize> {
    let q = QUERY.with_borrow(|q| q.to_lowercase());
    let mut apps = APPS.lock().unwrap();
    if q.is_empty() {
        // nothing typed: only what the user pinned (an app that is not a Start menu shortcut joins the list on demand)
        return PINNED
            .lock()
            .unwrap()
            .iter()
            .map(|p| {
                let w = wide(p);
                apps.iter().position(|a| a.1 == w).unwrap_or_else(|| {
                    let stem = std::path::Path::new(p).file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let name: String = stem.chars().take(1).flat_map(char::to_uppercase).chain(stem.chars().skip(1)).collect();
                    apps.push((name, w));
                    apps.len() - 1
                })
            })
            .collect();
    }
    // typed in the wrong layout: those matches come after the direct ones
    let fq = flip_layout(&q);
    let mut v: Vec<(u8, u32, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let n = a.0.to_lowercase();
            let s = score(&n, &q).or_else(|| score(&n, &fq).map(|s| s + 4))?;
            Some((s, u32::MAX - uses(&n), i))
        })
        .collect();
    v.sort_unstable(); // best match, then most launched, then alphabetical (the list is sorted by name)
    v.into_iter().map(|x| x.2).collect()
}

fn read_brightness() {
    std::thread::spawn(|| {
        let o = Command::new("powershell").args(["-NoProfile", "-Command", "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue).CurrentBrightness"]).creation_flags(NO_WINDOW).output();
        BRIGHT.store(o.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()).unwrap_or(-1), Relaxed);
    });
}

fn set_brightness(v: i32) {
    std::thread::spawn(move || {
        let c = format!("Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=1;Brightness={v}}}");
        let _ = Command::new("powershell").args(["-NoProfile", "-Command", &c]).creation_flags(NO_WINDOW).output();
    });
}

/// Reads what the sound panel shows: the devices, the microphone, the volume.
unsafe fn read_sound() {
    OUTS.set(audio::devices(false));
    INS.set(audio::devices(true));
    MIC.set(audio::mic().unwrap_or((-1, false)));
    read_volume();
}

/// Runs `f` with an open WLAN handle and the first wifi adapter. None when there is no wifi.
unsafe fn with_wlan<T>(f: impl FnOnce(HANDLE, &windows_sys::core::GUID) -> T) -> Option<T> {
    let (mut h, mut ver) = (null_mut(), 0u32);
    if WlanOpenHandle(2, null(), &mut ver, &mut h) != 0 {
        return None;
    }
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
    let r = if WlanEnumInterfaces(h, null(), &mut list) == 0 {
        let l = &*list;
        let r = if l.dwNumberOfItems > 0 { Some(f(h, &(*l.InterfaceInfo.as_ptr()).InterfaceGuid)) } else { None };
        WlanFreeMemory(list as *const _);
        r
    } else {
        None
    };
    WlanCloseHandle(h, null());
    r
}

unsafe fn read_nets() {
    let v = with_wlan(|h, g| {
        let mut out: Vec<Net> = Vec::new();
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = null_mut();
        if WlanGetAvailableNetworkList(h, g, 0, null(), &mut list) == 0 {
            let l = &*list;
            for i in 0..l.dwNumberOfItems as usize {
                let n = &*l.Network.as_ptr().add(i);
                let ssid = String::from_utf8_lossy(&n.dot11Ssid.ucSSID[..n.dot11Ssid.uSSIDLength as usize]).to_string();
                if ssid.is_empty() {
                    continue;
                }
                let pn = String::from_utf16_lossy(&n.strProfileName[..n.strProfileName.iter().position(|&c| c == 0).unwrap_or(0)]);
                let net = Net { ssid, q: n.wlanSignalQuality as i32, secure: n.bSecurityEnabled != 0, profile: (n.dwFlags & 2 != 0 && !pn.is_empty()).then_some(pn), connected: n.dwFlags & 1 != 0, auth: n.dot11DefaultAuthAlgorithm as i32 };
                match out.iter_mut().find(|o| o.ssid == net.ssid) {
                    Some(o) if o.q < net.q => *o = net,
                    Some(_) => {}
                    None => out.push(net),
                }
            }
            WlanFreeMemory(list as *const _);
        }
        out.sort_by_key(|n| (!n.connected, -n.q));
        out.truncate(7);
        let (mut size, mut data) = (0u32, null_mut());
        let conn = (WlanQueryInterface(h, g, wlan_intf_opcode_current_connection, null(), &mut size, &mut data, null_mut()) == 0).then(|| {
            let at = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
            let a = &at.wlanAssociationAttributes;
            let ssid = String::from_utf8_lossy(&a.dot11Ssid.ucSSID[..a.dot11Ssid.uSSIDLength as usize]).to_string();
            let c = Conn { ssid, auth: at.wlanSecurityAttributes.dot11AuthAlgorithm as i32, q: a.wlanSignalQuality as i32, rx: a.ulRxRate / 1000, tx: a.ulTxRate / 1000, phy: phy_name(a.dot11PhyType as i32) };
            WlanFreeMemory(data);
            c
        });
        // the network we are on can be missing from the scan list (hidden / not rescanned yet): show it anyway
        if let Some(c) = &conn {
            match out.iter().position(|n| n.ssid == c.ssid) {
                Some(i) => {
                    out[i].connected = true;
                    out.swap(0, i);
                }
                None => out.insert(0, Net { ssid: c.ssid.clone(), q: c.q, secure: c.auth != 1, profile: Some(c.ssid.clone()), connected: true, auth: c.auth }),
            }
        }
        (out, conn)
    });
    let (nets, conn) = v.unzip();
    NETS.set(nets.unwrap_or_default());
    CONN.set(conn.flatten());
}

unsafe fn wifi_connect(profile: &str) {
    let p = wide(profile);
    with_wlan(|h, g| {
        let params = WLAN_CONNECTION_PARAMETERS { wlanConnectionMode: wlan_connection_mode_profile, strProfile: p.as_ptr(), pDot11Ssid: null_mut(), pDesiredBssidList: null_mut(), dot11BssType: dot11_BSS_type_any, dwFlags: 0 };
        WlanConnect(h, g, &params, null());
    });
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Saves a profile for a new network and connects. shortcut: only open and WPA/WPA2/WPA3 personal (password) networks; enterprise ones go to Windows settings.
unsafe fn wifi_join(n: &Net, pass: &str) {
    let (auth, key) = match n.auth {
        1 => ("open", String::new()),
        4 => ("WPAPSK", pass.into()),
        7 => ("WPA2PSK", pass.into()),
        _ => ("WPA3SAE", pass.into()),
    };
    let sec = if auth == "open" {
        "<authEncryption><authentication>open</authentication><encryption>none</encryption><useOneX>false</useOneX></authEncryption>".to_string()
    } else {
        format!("<authEncryption><authentication>{auth}</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption><sharedKey><keyType>passphrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>", esc(&key))
    };
    let name = esc(&n.ssid);
    let xml = format!("<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{name}</name><SSIDConfig><SSID><name>{name}</name></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM><security>{sec}</security></MSM></WLANProfile>");
    let x = wide(&xml);
    let mut reason = 0u32;
    let ok = with_wlan(|h, g| WlanSetProfile(h, g, 0, x.as_ptr(), null(), 1, null_mut(), &mut reason) == 0).unwrap_or(false);
    if ok {
        wifi_connect(&n.ssid);
        NOTE.set(format!("Connecting to {}…", n.ssid));
    } else {
        NOTE.set("Could not save the network (wrong password format?)".into());
    }
}

// ---------- geometry ----------

fn size(k: Kind) -> (i32, i32) {
    match k {
        Kind::Quick => (360, 196 + if BRIGHT.load(Relaxed) >= 0 { 96 } else { 48 } + 64),
        Kind::Cal => (330, 340),
        Kind::Wifi => (360, wifi_rows().2),
        Kind::Sound => (360, sound_geo().h),
        Kind::Start => (560, 640),
        Kind::Menu => (MW, menu_rows().1),
    }
}

/// Wi-Fi panel rows in logical px: (top, bottom) per network (the connected one is a taller card),
/// y of the "Other networks" label, and the panel height.
fn wifi_rows() -> (Vec<(i32, i32)>, Option<i32>, i32) {
    NETS.with_borrow(|n| {
        let (mut y, mut rows, mut label) = (78 + ETH_CARD * eth_n(), vec![], None); // the wired connections come first
        for net in n.iter() {
            if net.connected {
                rows.push((y, y + 130));
                y += 140;
            } else {
                if label.is_none() {
                    label = Some(y);
                    y += 30;
                }
                rows.push((y, y + 48));
                y += 48;
            }
        }
        // an empty list leaves room for one line of text, unless a wired connection already says it all
        (rows, label, y + if !n.is_empty() || (!HAS_WIFI.get() && eth_n() > 0) { 8 } else { 84 } + 50)
    })
}

const ETH_CARD: i32 = 106; // height of a wired connection's card in the network panel, with the gap under it

/// Wired connections the network panel shows.
fn eth_n() -> i32 {
    ETHS.with_borrow(|e| e.len().min(2)) as i32
}

/// Sound panel rows in logical px: (top, bottom) per output, y of the "Input" label, y of the microphone's slider
/// (None without a microphone), (top, bottom) per input, and the panel height.
struct SoundGeo {
    outs: Vec<(i32, i32)>,
    in_label: i32,
    mic: Option<i32>,
    ins: Vec<(i32, i32)>,
    h: i32,
}

fn sound_geo() -> SoundGeo {
    let (no, ni) = (OUTS.with_borrow(|o| o.len().min(6)), INS.with_borrow(|i| i.len().min(6)));
    let mut y = 86;
    let rows = |n: usize, y: &mut i32| -> Vec<(i32, i32)> {
        let v = (0..n as i32).map(|i| (*y + 38 * i, *y + 38 * (i + 1))).collect();
        *y += if n == 0 { 30 } else { 38 * n as i32 }; // no devices: one line of text
        v
    };
    let outs = rows(no, &mut y);
    y += 12;
    let in_label = y;
    y += 26;
    let mic = (ni > 0 && MIC.get().0 >= 0).then(|| {
        y += 46;
        y - 46
    });
    let ins = rows(ni, &mut y);
    SoundGeo { outs, in_label, mic, ins, h: y + 10 + 50 }
}

fn r(l: i32, t: i32, rr: i32, b: i32) -> RECT {
    RECT { left: sc(l), top: sc(t), right: sc(rr), bottom: sc(b) }
}

fn rects(k: Kind) -> Vec<(RECT, Act)> {
    let (w, h) = size(k);
    match k {
        Kind::Quick => {
            // the network tile: with Wi-Fi its left part switches the radio and the arrow opens the list; on a PC without
            // Wi-Fi the whole tile opens the network panel. Bluetooth alike: switch + settings, or just settings without a radio.
            let mut v = if HAS_WIFI.get() { vec![(r(16, 16, 84, 72), Act::Tile(0)), (r(84, 16, 120, 72), Act::WifiMore)] } else { vec![(r(16, 16, 120, 72), Act::WifiMore)] };
            if net::BT.load(Relaxed) >= 0 {
                v.extend([(r(128, 16, 196, 72), Act::Tile(1)), (r(196, 16, 232, 72), Act::BtMore)]);
            } else {
                v.push((r(128, 16, 232, 72), Act::BtMore));
            }
            v.extend((2..6u8).map(|i| {
                let (c, rw) = (i as i32 % 3, i as i32 / 3);
                (r(16 + 112 * c, 16 + 96 * rw, 16 + 112 * c + 104, 72 + 96 * rw), Act::Tile(i))
            }));
            let mut y = 196;
            if BRIGHT.load(Relaxed) >= 0 {
                v.push((r(60, y, w - 16, y + 40), Act::BrightSlide));
                y += 48;
            }
            v.push((r(16, y, 52, y + 40), Act::Mute));
            v.push((r(60, y, w - 52, y + 40), Act::VolSlide));
            v.push((r(w - 48, y, w - 12, y + 40), Act::SoundMore));
            v.push((r(w - 52, h - 48, w - 12, h - 8), Act::Settings("ms-settings:")));
            v
        }
        Kind::Sound => {
            let g = sound_geo();
            let mut v = vec![(r(16, 40, 52, 80), Act::Mute), (r(60, 40, w - 16, 80), Act::VolSlide)];
            v.extend(g.outs.iter().enumerate().map(|(i, &(t, b))| (r(12, t, w - 12, b), Act::OutDev(i))));
            if let Some(m) = g.mic {
                v.extend([(r(16, m, 52, m + 40), Act::MicMute), (r(60, m, w - 16, m + 40), Act::MicSlide)]);
            }
            v.extend(g.ins.iter().enumerate().map(|(i, &(t, b))| (r(12, t, w - 12, b), Act::InDev(i))));
            v.push((r(12, h - 48, w / 2 + 16, h - 12), Act::Settings("ms-settings:sound")));
            v.push((r(w / 2 + 20, h - 48, w - 12, h - 12), Act::Mixer));
            v
        }
        Kind::Cal => vec![(r(w - 84, 10, w - 52, 42), Act::CalPrev), (r(w - 48, 10, w - 16, 42), Act::CalNext), (r(16, 10, w - 100, 42), Act::CalToday)],
        Kind::Wifi => {
            let rows = wifi_rows().0;
            let mut v: Vec<_> = rows.iter().enumerate().map(|(i, &(t, b))| (r(12, t, w - 12, b), Act::Net(i))).collect();
            if let Some(i) = NETS.with_borrow(|n| n.iter().position(|n| n.connected)) {
                let b = rows[i].1;
                v.push((r(w - 24 - 116, b - 42, w - 24, b - 12), Act::NetOff));
            }
            if HAS_WIFI.get() {
                v.push((r(w - 66, 24, w - 18, 48), Act::WifiSwitch));
            }
            v.push((r(12, h - 48, w - 12, h - 12), Act::Settings(if HAS_WIFI.get() { "ms-settings:network-wifi" } else { "ms-settings:network-ethernet" })));
            v
        }
        Kind::Menu => menu_rows().0.into_iter().enumerate().map(|(i, (t, b))| (r(6, t, w - 6, b), Act::Menu(i as u8))).collect(),
        Kind::Start => {
            let f = filtered();
            let s = SCROLL.get();
            let mut v: Vec<_> = (0..ROWS).filter(|i| s + i < f.len()).map(|i| (r(12, 76 + 40 * i as i32, w - 12, 76 + 40 * (i as i32 + 1)), Act::App(s + i))).collect();
            let pw = power_shown();
            let px = w - 16 - 44 * pw.len() as i32;
            v.extend(pw.iter().enumerate().map(|(i, &p)| (r(px + 44 * i as i32, h - 52, px + 44 * (i as i32 + 1) - 4, h - 12), Act::Power(p))));
            v
        }
    }
}

fn act_at(k: Kind, x: i32, y: i32) -> Option<(RECT, Act)> {
    rects(k).into_iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

// ---------- open / close ----------

pub fn init(hi: HINSTANCE) {
    pinned_load();
    unsafe {
        let class = wide("topbar_fly");
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: class.as_ptr(), ..zeroed() };
        RegisterClassW(&wc);
    }
}

/// One row of a menu flyout: a label (with a window's icon when it stands for a window), a dim keyboard hint on the right,
/// a dot when it is the current choice, red when it destroys something, a separator line above it when `sep`.
pub struct MItem {
    pub icon: HICON,
    pub label: String,
    pub hint: String,
    pub on: bool,
    pub danger: bool,
    pub sep: bool,
}

impl MItem {
    pub fn new(label: impl Into<String>) -> Self {
        MItem { icon: null_mut(), label: label.into(), hint: String::new(), on: false, danger: false, sep: false }
    }
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = h.into();
        self
    }
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn sep(mut self) -> Self {
        self.sep = true;
        self
    }
}

type Pick = Box<dyn FnOnce(usize)>;
thread_local! {
    static MITEMS: RefCell<Vec<MItem>> = const { RefCell::new(Vec::new()) };
    static MHEAD: RefCell<Option<(HICON, &'static str, String, String)>> = const { RefCell::new(None) }; // icon (or a glyph when null), title, subtitle
    static MPICK: RefCell<Option<Pick>> = const { RefCell::new(None) };
}
const MW: i32 = 272; // menu width
const MROW: i32 = 38;
const MHEAD_H: i32 = 58;

/// Menu flyout above the bar, centered on screen x `cx`. `pick` gets the index of the chosen item (not called on cancel).
/// A right click while a menu is open replaces it.
pub fn menu(cx: i32, head: Option<(HICON, &'static str, String, String)>, items: Vec<MItem>, pick: impl FnOnce(usize) + 'static) {
    if is_open(Kind::Menu) {
        close_now();
    }
    LAST.set((None, 0)); // always opens, even right after another menu closed
    MITEMS.set(items);
    MHEAD.set(head);
    MPICK.set(Some(Box::new(pick)));
    MENU_X.set(cx - sc(MW) / 2);
    toggle(Kind::Menu);
}

/// (top, bottom) of each menu item in logical px, and the panel height.
fn menu_rows() -> (Vec<(i32, i32)>, i32) {
    let mut y = if MHEAD.with_borrow(|h| h.is_some()) { MHEAD_H + 9 } else { 6 };
    let rows = MITEMS.with_borrow(|m| {
        m.iter()
            .map(|it| {
                if it.sep {
                    y += 9;
                }
                y += MROW;
                (y - MROW, y)
            })
            .collect()
    });
    (rows, y + 6)
}

/// Right-click menu of an empty spot of the bar (and of the Start button), centered on screen x `cx`. The bar's own
/// settings are submenus here: they replace this menu in place.
pub fn context_menu(cx: i32) {
    #[derive(Clone, Copy)]
    enum A {
        Settings,
        Personal,
        Bar,
        Start,
        Style,
        TaskMgr,
        Cleaner,
        Purge,
        Lang,
        Quit,
    }
    let mut items = vec![
        (MItem::new(t!("Параметры Windows", "Windows Settings")).hint("Win+I"), A::Settings),
        (MItem::new(t!("Персонализация", "Personalization")), A::Personal),
        (MItem::new(t!("Настройки панели", "Bar settings")).hint("›").sep(), A::Bar),
        (MItem::new(t!("Меню «Пуск»", "Start menu")).hint("›"), A::Start),
        (MItem::new(t!("Стиль", "Style")).hint("›"), A::Style),
        (MItem::new(t!("Диспетчер задач", "Task Manager")).hint("Ctrl+Shift+Esc").sep(), A::TaskMgr),
        (MItem::new(t!("Чистильщик диска", "Disk Cleaner")), A::Cleaner),
    ];
    if crate::purge_installed() {
        items.push((MItem::new(t!("Удалить мусор Windows (необратимо)", "Remove Windows junk (irreversible)")).danger(), A::Purge));
    }
    items.push((MItem::new(t!("Программы WinRust на English", "WinRust apps in Русский")), A::Lang)); // shared with the other WinRust programs
    items.push((MItem::new(t!("Закрыть панель", "Close the bar")).danger().sep(), A::Quit));
    let acts: Vec<A> = items.iter().map(|x| x.1).collect();
    menu(cx, None, items.into_iter().map(|x| x.0).collect(), move |i| unsafe {
        match acts.get(i).copied() {
            Some(A::Settings) => open(&wide("ms-settings:")),
            Some(A::Personal) => open(&wide("ms-settings:personalization")),
            Some(A::Bar) => crate::settings_menu(cx),
            Some(A::Start) => crate::start_menu(cx),
            Some(A::Style) => crate::style_menu(cx),
            // with rtaskmgr-win32 installed this is our task manager; a full path because explorer.exe does not search PATH
            Some(A::TaskMgr) => open(&wide(&format!(r"{}\System32\taskmgr.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())))),
            Some(A::Cleaner) => open_cleaner(),
            Some(A::Purge) => crate::open_purge(),
            Some(A::Lang) => crate::i18n::set_english(!crate::i18n::english()),
            Some(A::Quit) => _ = PostMessageW(FindWindowW(wide("topbar").as_ptr(), null()), WM_CLOSE, 0, 0), // quits the bar, brings the Windows taskbar back
            None => {}
        }
    });
}

/// Starts the fade-out of the open flyout (it stays as a ghost window until the fade ends).
pub fn close() {
    if FLY.get().is_some() {
        LAST.set((FLY.get(), unsafe { now_ms() }));
    }
    let h = FH.replace(null_mut());
    FLY.set(None);
    if h.is_null() {
        return;
    }
    unsafe { ShowWindow(TIPWND.get(), SW_HIDE) }; // a button's tooltip goes with its panel
    unsafe {
        kill_ghost();
        GHOST.set((h, now_ms(), FIN.get()));
        KillTimer(h, 1);
        KillTimer(h, 2);
        SetTimer(h, 3, 15, None);
    }
}

/// Closes without the fade (another flyout replaces it, or a fullscreen app needs the screen).
pub fn close_now() {
    close();
    unsafe { kill_ghost() };
}

unsafe fn kill_ghost() {
    let (g, _, _) = GHOST.replace((null_mut(), 0, (0, 0)));
    if !g.is_null() {
        DestroyWindow(g);
    }
}

/// One frame of a flyout fade: opening rises up from the bar while fading in, closing fades out while sinking back down.
unsafe fn anim_step(hw: HWND) {
    let now = now_ms();
    let (g, gt0, gpos) = GHOST.get();
    if hw == g {
        let t = (now.saturating_sub(gt0) as f32 / 150.0).min(1.0);
        let q = t * t;
        SetLayeredWindowAttributes(hw, 0, ((1.0 - q) * 255.0) as u8, LWA_ALPHA);
        SetWindowPos(hw, null_mut(), gpos.0, gpos.1 + (q * sc(10) as f32) as i32, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        if t >= 1.0 {
            kill_ghost();
        }
    } else if hw == FH.get() {
        let t = (now.saturating_sub(T0.get()) as f32 / 220.0).min(1.0);
        let p = 1.0 - (1.0 - t).powi(3);
        SetLayeredWindowAttributes(hw, 0, (p * 255.0) as u8, LWA_ALPHA);
        let (x, y) = FIN.get();
        SetWindowPos(hw, null_mut(), x, y + ((1.0 - p) * sc(14) as f32) as i32, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        if t >= 1.0 {
            KillTimer(hw, 3);
        }
    } else {
        KillTimer(hw, 3);
    }
}

pub fn toggle(k: Kind) {
    if FLY.get() == Some(k) {
        return close();
    }
    let (was, at) = LAST.get();
    if was == Some(k) && unsafe { now_ms() }.saturating_sub(at) < 400 {
        return; // it closed an instant ago (focus moved to the key / click that is toggling it): this press means "close", not "reopen"
    }
    close();
    unsafe {
        HV.with_borrow_mut(|m| m.retain(|k, _| *k < 1_000_000)); // flyout keys are >= 1_000_000: no stale glow from the last panel
        QUERY.set(String::new());
        SCROLL.set(0);
        SEL.set(0);
        PASS.set(None);
        NOTE.set(String::new());
        CONFIRM.set(-1);
        OVER.set(None);
        let mut t: SYSTEMTIME = zeroed();
        GetLocalTime(&mut t);
        CAL.set((t.wYear as i32, t.wMonth as i32));
        match k {
            Kind::Wifi => {
                with_wlan(|h, g| WlanScan(h, g, null(), null(), null()));
                read_nets();
                read_wifi(); // also the wired connections
            }
            Kind::Quick => {
                read_brightness();
                read_nets();
                read_wifi();
            }
            Kind::Sound => {
                audio::reset();
                read_sound();
            }
            Kind::Start => {
                pinned_load(); // the search card (rs-launcher) can add to this list from another process
                std::thread::spawn(load_apps); // apps installed since the last open show up (the panel repaints every second)
            }
            _ => {}
        }
        let (w, h) = (sc(size(k).0), sc(size(k).1));
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let x = match k {
            Kind::Start => (sw - w) / 2,
            Kind::Menu => MENU_X.get().min(sw - w - sc(8)).max(sc(8)),
            _ => sw - w - sc(8),
        };
        let class = wide("topbar_fly");
        let fy = pop_y(h); // above the bar (below it at the top of the screen)
        let hw = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED, class.as_ptr(), null(), WS_POPUP, x, fy + sc(14), w, h, null_mut(), null_mut(), GetModuleHandleW(null()), null());
        SetLayeredWindowAttributes(hw, 0, 0, LWA_ALPHA); // starts invisible, anim_step fades it in
        if k == Kind::Start {
            // accept file drops (an app dragged onto the panel is added to Start); the bar may be elevated, so lift UIPI for them
            DragAcceptFiles(hw, 1);
            for m in [WM_DROPFILES, 0x49 /* WM_COPYGLOBALDATA */, WM_COPYDATA] {
                ChangeWindowMessageFilterEx(hw, m, MSGFLT_ALLOW, null_mut());
            }
        }
        FIN.set((x, fy));
        T0.set(now_ms());
        SetWindowRgn(hw, CreateRoundRectRgn(0, 0, w + 1, h + 1, rr(16), rr(16)), 1);
        FH.set(hw);
        FLY.set(Some(k));
        ShowWindow(hw, SW_SHOW);
        force_fg(hw);
        SetTimer(hw, 1, 1000, None);
        SetTimer(hw, 3, 15, None);
        if k == Kind::Quick {
            net::bt_read(hw as isize); // the Bluetooth tile fills in when the radio has answered
        }
    }
}

// ---------- actions ----------

unsafe fn slide(a: Act, x: i32, rect: RECT) {
    let f = ((x - rect.left - sc(8)) as f32 / (rect.right - rect.left - sc(16)) as f32).clamp(0.0, 1.0);
    match a {
        Act::VolSlide => set_volume((f * 100.0).round() as i32),
        Act::MicSlide => {
            audio::set_mic((f * 100.0).round() as i32);
            MIC.set(((f * 100.0).round() as i32, false));
        }
        _ => BRIGHT.store(((f * 100.0) as i32).max(1), Relaxed),
    }
}

/// Wi-Fi radio on/off (the Wi-Fi tile). shortcut: software radio of the first adapter / first PHY only.
unsafe fn wifi_radio(on: bool) {
    with_wlan(|h, g| {
        let s = WLAN_PHY_RADIO_STATE { dwPhyIndex: 0, dot11SoftwareRadioState: if on { dot11_radio_state_on } else { dot11_radio_state_off }, dot11HardwareRadioState: dot11_radio_state_on };
        WlanSetInterface(h, g, wlan_intf_opcode_radio_state, size_of::<WLAN_PHY_RADIO_STATE>() as u32, &s as *const _ as *const _, null());
    });
}

/// Starts the i-th app of the filtered Start list and counts the launch (search ranks by it). false = no such row.
unsafe fn launch(i: usize) -> bool {
    let Some(app) = filtered().get(i).and_then(|&ai| APPS.lock().unwrap().get(ai).cloned()) else { return false };
    if wstr(&app.1).starts_with("shell:") {
        bump_name(&app.0); // open() counts only .lnk files
    }
    open(&app.1);
    true
}

/// Right-click menu of a Start app: open, open as administrator, show the shortcut's folder.
unsafe fn app_menu(hw: HWND, i: usize, x: i32, y: i32) {
    let Some(lnk) = filtered().get(i).and_then(|&ai| APPS.lock().unwrap().get(ai).map(|a| a.1.clone())) else { return };
    let m = CreatePopupMenu();
    let path = wstr(&lnk);
    let file = !path.starts_with("shell:"); // Store apps have no file to run elevated or to show in a folder
    for (id, label) in [(1, "Open"), (2, "Run as administrator"), (3, "Open file location"), (4, if start_pinned(&path) { "Remove from Start" } else { "Add to Start" })] {
        if file || id == 1 || id == 4 {
            AppendMenuW(m, MF_STRING, id, wide(label).as_ptr());
        }
    }
    let mut pt = POINT { x, y };
    ClientToScreen(hw, &mut pt);
    let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hw, null());
    DestroyMenu(m);
    match cmd {
        1 => _ = launch(i),
        2 => {
            ShellExecuteW(null_mut(), wide("runas").as_ptr(), lnk.as_ptr(), null(), null(), SW_SHOWNORMAL);
        }
        3 => {
            let p = String::from_utf16_lossy(&lnk[..lnk.len().saturating_sub(1)]);
            ShellExecuteW(null_mut(), wide("open").as_ptr(), wide("explorer.exe").as_ptr(), wide(&format!("/select,\"{p}\"")).as_ptr(), null(), SW_SHOWNORMAL);
        }
        4 => {
            start_toggle(&path);
            return; // stay open, the list just changed
        }
        _ => return,
    }
    close();
}

unsafe fn run_act(a: Act, rect: RECT, x: i32) {
    match a {
        Act::VolSlide | Act::BrightSlide | Act::MicSlide => {
            DRAG.set(match a {
                Act::VolSlide => 1,
                Act::BrightSlide => 2,
                _ => 3,
            });
            SetCapture(FH.get());
            slide(a, x, rect);
        }
        Act::WifiMore => toggle(Kind::Wifi),
        Act::SoundMore => toggle(Kind::Sound),
        Act::OutDev(i) | Act::InDev(i) => {
            let id = if matches!(a, Act::OutDev(_)) { &OUTS } else { &INS }.with_borrow(|d| d.get(i).filter(|d| !d.default).map(|d| d.id.clone()));
            if let Some(id) = id {
                if audio::set_default(&id) {
                    EP.set(None); // the bar's volume now belongs to the new device
                } else {
                    NOTE.set(t!("Windows не дала сменить устройство", "Windows would not switch the device").into());
                }
                read_sound();
            }
        }
        Act::MicMute => {
            audio::toggle_mic();
            MIC.set(audio::mic().unwrap_or((-1, false)));
        }
        Act::Mixer => {
            open(&wide(&format!(r"{}\System32\SndVol.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))));
            close();
        }
        Act::BtMore => {
            open(&wide("ms-settings:bluetooth"));
            close();
        }
        Act::WifiSwitch => wifi_radio(WIFI.get() < 0 && NETS.with_borrow(|n| n.is_empty())),
        Act::Tile(i) => match i {
            0 => wifi_radio(WIFI.get() < 0),
            1 => net::bt_set(net::BT.load(Relaxed) != 1, FH.get() as isize),
            5 => {
                close();
                keys(&[VK_LWIN, VK_CONTROL, b'L' as u16]); // live captions
            }
            _ => {
                open(&wide(["", "ms-settings:bluetooth", "ms-settings:network-airplanemode", "ms-settings:easeofaccess", "ms-settings:batterysaver"][i as usize]));
                close();
            }
        },
        Act::Mute => toggle_mute(),
        Act::Menu(i) => {
            close();
            if let Some(pick) = MPICK.take() {
                pick(i as usize);
            }
        }
        Act::Settings(u) => {
            open(&wide(u));
            close();
        }
        Act::CalPrev | Act::CalNext => {
            let (mut y, mut m) = CAL.get();
            m += if a == Act::CalNext { 1 } else { -1 };
            (y, m) = if m < 1 { (y - 1, 12) } else if m > 12 { (y + 1, 1) } else { (y, m) };
            CAL.set((y, m));
        }
        Act::CalToday => {
            let mut t: SYSTEMTIME = zeroed();
            GetLocalTime(&mut t);
            CAL.set((t.wYear as i32, t.wMonth as i32));
        }
        Act::Net(i) => {
            let n = NETS.with_borrow(|n| n.get(i).map(|n| (n.connected, n.profile.clone(), n.auth, n.secure, n.ssid.clone())));
            let Some((connected, profile, auth, secure, ssid)) = n else { return };
            if connected {
                // the card is information; its Disconnect button is the action
            } else if let Some(p) = profile {
                wifi_connect(&p);
                NOTE.set(tf!("Подключение к {ssid}…", "Connecting to {ssid}…"));
            } else if !secure {
                NETS.with_borrow(|l| wifi_join(&l[i], ""));
            } else if matches!(auth, 4 | 7 | 8 | 9) {
                PASS.set(Some((i, String::new())));
            } else {
                open(&wide("ms-settings:network-wifi"));
                close();
            }
        }
        Act::NetOff => {
            with_wlan(|h, g| WlanDisconnect(h, g, null()));
            NOTE.set(t!("Отключено", "Disconnected").into());
        }
        Act::App(i) => {
            if launch(i) {
                close();
            }
        }
        Act::Power(POWER_MENU) => {
            let cx = FIN.get().0 + (rect.left + rect.right) / 2;
            close();
            power_menu(cx);
        }
        Act::Power(p) => {
            if power_confirm(p) && CONFIRM.get() != p as i32 {
                CONFIRM.set(p as i32); // first click only arms it
                return;
            }
            close();
            power(p);
        }
    }
}

/// Sleep (S3 / modern standby), never hibernate. SetSuspendState needs the shutdown privilege switched on in our token.
fn sleep_now() {
    use windows_sys::Win32::{
        Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES},
        System::{Power::SetSuspendState, Threading::{GetCurrentProcess, OpenProcessToken}},
    };
    unsafe {
        let mut tok = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut tok) != 0 {
            let mut tp: TOKEN_PRIVILEGES = zeroed();
            tp.PrivilegeCount = 1;
            tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            if LookupPrivilegeValueW(null(), wide("SeShutdownPrivilege").as_ptr(), &mut tp.Privileges[0].Luid) != 0 {
                AdjustTokenPrivileges(tok, 0, &tp, 0, null_mut(), null_mut());
            }
            CloseHandle(tok);
        }
        SetSuspendState(false, false, false);
    }
}

// ---------- painting ----------

unsafe fn track(dc: HDC, rc: RECT, frac: f32) {
    let (l, rr, m) = (rc.left + sc(8), rc.right - sc(8), (rc.top + rc.bottom) / 2);
    fill(dc, RECT { left: l, top: m - sc(2), right: rr, bottom: m + sc(2) }, mix(pal().line, pal().dim, 0.3), sc(4));
    let x = l + ((rr - l) as f32 * frac.clamp(0.0, 1.0)) as i32;
    fill(dc, RECT { left: l, top: m - sc(2), right: x, bottom: m + sc(2) }, pal().accent, sc(4));
    fill(dc, RECT { left: x - sc(8), top: m - sc(8), right: x + sc(8), bottom: m + sc(8) }, pal().text, sc(16));
}

unsafe fn paint(hw: HWND, k: Kind) {
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(hw, &mut ps);
    let mut cr: RECT = zeroed();
    GetClientRect(hw, &mut cr);
    BUSY.set(false);
    let (dc, bmp) = (CreateCompatibleDC(wdc), CreateCompatibleBitmap(wdc, cr.right, cr.bottom));
    let old = SelectObject(dc, bmp);
    fill(dc, cr, pal().line, sc(16));
    fill(dc, RECT { left: 1, top: 1, right: cr.right - 1, bottom: cr.bottom - 1 }, pal().bg, sc(16));
    SetBkMode(dc, TRANSPARENT as i32);
    let (face, gface) = (wide(ui_face()), wide("Segoe MDL2 Assets"));
    let font = |f: &[u16], px: i32, wt: i32| CreateFontW(-sc(px), 0, 0, 0, wt, 0, 0, 0, 1, 0, 0, 5, 0, f.as_ptr());
    let (f_txt, f_small, f_glyph, f_title, f_big) = (font(&face, 13, 400), font(&face, 12, 400), font(&gface, 16, 400), font(&face, 15, 600), font(&face, 34, 600));
    let of = SelectObject(dc, f_txt);
    let (w, h) = size(k);
    let over = OVER.get();
    let hot = |a: Act| over == Some(a);
    // hover highlight that fades; the key is the element's position (flyout keys start at 1_000_000)
    let g = |a: Act, rc: RECT, from: u32, to: u32, rad: i32| glow(dc, rc.top * 4096 + rc.left + 1_000_000, hot(a), rc, from, to, rad);
    let (vol, muted) = VOL.get();

    match k {
        Kind::Quick => {
            let rs = rects(k);
            let (state, wifi, bt) = (net(), HAS_WIFI.get(), net::BT.load(Relaxed));
            // the network tile: the Wi-Fi network, or on a PC without Wi-Fi the cable's state
            let (net_label, net_on) = if wifi {
                (NETS.with_borrow(|n| n.iter().find(|n| n.connected).map(|n| n.ssid.clone())).unwrap_or("Wi‑Fi".into()), WIFI.get() >= 0)
            } else {
                match state {
                    Link::Cable => (ETHS.with_borrow(|e| e.first().map(|e| e.name.clone())).unwrap_or(t!("Сеть", "Network").into()), true),
                    Link::CableNoInternet => (t!("Нет интернета", "No internet").into(), true),
                    _ => (t!("Не подключено", "Not connected").to_string(), false),
                }
            };
            // Bluetooth: what is connected, when something is
            let bt_label = match net::BT_DEVS.lock().unwrap().as_slice() {
                [] => "Bluetooth".to_string(),
                [one] => one.clone(),
                [first, rest @ ..] => format!("{first} +{}", rest.len()),
            };
            // (glyph, label, lit, has an arrow part)
            let tiles = [("", net_label, net_on, wifi), ("\u{E702}", bt_label, bt == 1, bt >= 0), ("\u{E709}", "Airplane mode".into(), false, false), ("\u{E776}", "Accessibility".into(), false, false), ("\u{E945}", "Energy saver".into(), false, false), ("\u{E7F0}", "Live captions".into(), false, false)];
            for (i, (glyph, label, on, split)) in tiles.iter().enumerate() {
                let (on, split) = (*on, *split);
                let c = (i % 3) as i32;
                let (tl, tt) = (16 + 112 * c, 16 + 96 * (i as i32 / 3));
                let full = r(tl, tt, tl + 104, tt + 56);
                let (base, ink) = (if on { pal().accent } else { mix(pal().bg, pal().line, 0.5) }, if on { pal().bg } else { pal().text });
                fill(dc, full, if on { pal().accent } else { pal().line }, sc(8));
                fill(dc, RECT { left: full.left + 1, top: full.top + 1, right: full.right - 1, bottom: full.bottom - 1 }, base, sc(8));
                for (rc, a) in rs.iter().filter(|(rc, _)| rc.top == full.top && rc.bottom == full.bottom && rc.left >= full.left && rc.right <= full.right) {
                    g(*a, *rc, base, if on { mix(pal().accent, pal().text, 0.3) } else { pal().hover }, sc(8));
                }
                let gr = if split { r(tl, tt, tl + 68, tt + 56) } else { full };
                let (gx, gy) = ((gr.left + gr.right) / 2, (gr.top + gr.bottom) / 2);
                if i == 0 && wifi {
                    // the same arcs as the bar: lit by the signal, a cross when not connected
                    let q = WIFI.get();
                    wifi_glyph(dc, gx, gy, sc(20), if q >= 0 { arcs_lit(q) } else { 0 }, ink, mix(ink, base, 0.55), if q < 0 { 1 } else { 0 });
                } else if i == 0 {
                    eth_glyph(dc, gx, gy, sc(20), f_glyph, ink, ink, match state {
                        Link::Cable => 0,
                        Link::CableNoInternet => 2,
                        _ => 1,
                    });
                } else {
                    SelectObject(dc, f_glyph);
                    text(dc, glyph, gr, ink, DT_CENTER);
                }
                if split {
                    fill(dc, r(tl + 68, tt + 12, tl + 69, tt + 44), if on { pal().bg } else { pal().line }, 0);
                    SelectObject(dc, f_glyph);
                    text(dc, "\u{E76C}", r(tl + 68, tt, tl + 104, tt + 56), ink, DT_CENTER);
                }
                SelectObject(dc, f_small);
                text(dc, label, r(tl, tt + 60, tl + 104, tt + 82), pal().text, DT_CENTER);
            }
            let mut y = 196;
            let b = BRIGHT.load(Relaxed);
            if b >= 0 {
                SelectObject(dc, f_glyph);
                text(dc, "\u{E706}", r(16, y, 52, y + 40), pal().text, DT_CENTER);
                track(dc, rs.iter().find(|(_, a)| *a == Act::BrightSlide).unwrap().0, b as f32 / 100.0);
                y += 48;
            }
            g(Act::Mute, r(16, y, 52, y + 40), pal().bg, pal().hover, sc(8));
            SelectObject(dc, f_glyph);
            text(dc, if muted || vol == 0 { "\u{E74F}" } else { "\u{E767}" }, r(16, y, 52, y + 40), pal().text, DT_CENTER);
            track(dc, rs.iter().find(|(_, a)| *a == Act::VolSlide).unwrap().0, if muted { 0.0 } else { vol.max(0) as f32 / 100.0 });
            // the arrow: which device plays, and the microphone
            g(Act::SoundMore, r(w - 48, y, w - 12, y + 40), pal().bg, pal().hover, sc(8));
            SelectObject(dc, f_glyph);
            text(dc, "\u{E76C}", r(w - 48, y, w - 12, y + 40), pal().text, DT_CENTER);
            fill(dc, r(0, h - 56, w, h - 55), pal().line, 0);
            let (pct, ac) = BAT.get();
            SelectObject(dc, f_glyph);
            if pct >= 0 {
                text(dc, if ac { "\u{E83E}" } else { "\u{E83F}" }, r(16, h - 48, 44, h - 8), if ac { GOOD } else { pal().text }, DT_CENTER);
                SelectObject(dc, f_txt);
                text(dc, &format!("{pct}%"), r(48, h - 48, 140, h - 8), pal().text, DT_LEFT);
            }
            g(Act::Settings("ms-settings:"), r(w - 52, h - 48, w - 12, h - 8), pal().bg, pal().hover, sc(8));
            SelectObject(dc, f_glyph);
            text(dc, "\u{E713}", r(w - 52, h - 48, w - 12, h - 8), pal().text, DT_CENTER);
        }
        Kind::Sound => {
            let (rs, geo) = (rects(k), sound_geo());
            let find = |a: Act| rs.iter().find(|(_, x)| *x == a).map(|x| x.0);
            let label = |s: &str, y: i32| {
                SelectObject(dc, f_small);
                text(dc, s, r(18, y, w - 16, y + 22), pal().dim, DT_LEFT);
            };
            // one device row: a check mark and brighter text for the one in use
            let row = |a: Act, d: &audio::Dev| {
                let Some(rc) = find(a) else { return };
                g(a, rc, pal().bg, pal().hover, sc(8));
                if d.default {
                    SelectObject(dc, f_glyph);
                    text(dc, "\u{E73E}", RECT { left: rc.left + sc(6), right: rc.left + sc(34), ..rc }, pal().accent, DT_CENTER);
                }
                SelectObject(dc, f_txt);
                text(dc, &d.name, RECT { left: rc.left + sc(40), right: rc.right - sc(10), ..rc }, if d.default { pal().text } else { mix(pal().dim, pal().text, 0.55) }, DT_LEFT);
            };
            label(t!("Вывод", "Output"), 14);
            g(Act::Mute, r(16, 40, 52, 80), pal().bg, pal().hover, sc(8));
            SelectObject(dc, f_glyph);
            text(dc, if muted || vol == 0 { "\u{E74F}" } else { "\u{E767}" }, r(16, 40, 52, 80), if muted { BAD } else { pal().text }, DT_CENTER);
            track(dc, r(60, 40, w - 16, 80), if muted { 0.0 } else { vol.max(0) as f32 / 100.0 });
            OUTS.with_borrow(|o| {
                o.iter().take(6).enumerate().for_each(|(i, d)| row(Act::OutDev(i), d));
                if o.is_empty() {
                    SelectObject(dc, f_txt);
                    text(dc, t!("Нет устройств вывода", "No output devices"), r(18, 86, w - 16, 116), pal().dim, DT_LEFT);
                }
            });
            fill(dc, r(16, geo.in_label - 6, w - 16, geo.in_label - 5), pal().line, 0);
            label(t!("Ввод", "Input"), geo.in_label);
            if let Some(m) = geo.mic {
                let (lvl, off) = MIC.get();
                g(Act::MicMute, r(16, m, 52, m + 40), pal().bg, pal().hover, sc(8));
                SelectObject(dc, f_glyph);
                text(dc, "\u{E720}", r(16, m, 52, m + 40), if off { BAD } else { pal().text }, DT_CENTER);
                track(dc, r(60, m, w - 16, m + 40), if off { 0.0 } else { lvl.max(0) as f32 / 100.0 });
            }
            INS.with_borrow(|v| {
                v.iter().take(6).enumerate().for_each(|(i, d)| row(Act::InDev(i), d));
                if v.is_empty() {
                    SelectObject(dc, f_txt);
                    text(dc, t!("Нет микрофона", "No microphone"), r(18, geo.in_label + 26, w - 16, geo.in_label + 56), pal().dim, DT_LEFT);
                }
            });
            fill(dc, r(0, h - 56, w, h - 55), pal().line, 0);
            let note = NOTE.with_borrow(|n| n.clone());
            for (a, words) in [(Act::Settings("ms-settings:sound"), t!("Параметры звука", "Sound settings")), (Act::Mixer, t!("Микшер громкости", "Volume mixer"))] {
                let Some(rc) = find(a) else { continue };
                g(a, rc, pal().bg, pal().hover, sc(8));
                SelectObject(dc, f_txt);
                // a refused device switch is said where the eye is, in place of the first link
                let failed = !note.is_empty() && a != Act::Mixer;
                text(dc, if failed { &note } else { words }, RECT { left: rc.left + sc(10), ..rc }, if failed { WARN } else { pal().accent }, DT_LEFT);
            }
        }
        Kind::Cal => {
            let (y, m) = CAL.get();
            SelectObject(dc, f_title);
            text(dc, &format!("{} {y}", MONTHS[m as usize - 1]), r(16, 10, w - 100, 42), pal().text, DT_LEFT);
            SelectObject(dc, f_glyph);
            text(dc, "\u{E70E}", r(w - 84, 10, w - 52, 42), pal().text, DT_CENTER);
            text(dc, "\u{E70D}", r(w - 48, 10, w - 16, 42), pal().text, DT_CENTER);
            SelectObject(dc, f_small);
            for (i, d) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().enumerate() {
                text(dc, d, r(18 + 42 * i as i32, 54, 60 + 42 * i as i32, 78), pal().dim, DT_CENTER);
            }
            let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
            let yy = if m < 3 { y - 1 } else { y };
            let first = ((yy + yy / 4 - yy / 100 + yy / 400 + t[m as usize - 1] + 1) % 7 + 6) % 7; // Monday = 0
            let dim = match m {
                2 => if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 },
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            let mut now: SYSTEMTIME = zeroed();
            GetLocalTime(&mut now);
            SelectObject(dc, f_txt);
            for d in 1..=dim {
                let (col, row) = ((first + d - 1) % 7, (first + d - 1) / 7);
                let cell = r(18 + 42 * col, 84 + 40 * row, 60 + 42 * col, 120 + 40 * row);
                let today = now.wYear as i32 == y && now.wMonth as i32 == m && now.wDay as i32 == d;
                if today {
                    fill(dc, cell, pal().accent, sc(18));
                }
                text(dc, &d.to_string(), cell, if today { pal().bg } else { pal().text }, DT_CENTER);
            }
        }
        Kind::Wifi => {
            let on = NETS.with_borrow(|n| !n.is_empty()) || WIFI.get() >= 0;
            let state = net();
            // header: the live network icon, the title and one line of plain status
            let wifi = HAS_WIFI.get(); // a PC without Wi-Fi gets a network panel about its cable, with no radio switch
            net_icon(dc, sc(34), sc(40), sc(26), f_glyph);
            SelectObject(dc, f_title);
            text(dc, if wifi { "Wi‑Fi" } else { t!("Сеть", "Network") }, r(64, 18, w - 90, 40), pal().text, DT_LEFT);
            let note = NOTE.with_borrow(|n| n.clone());
            let (sub, sub_col) = if !note.is_empty() {
                (note, pal().dim)
            } else {
                match state {
                    Link::Wifi(_) => (t!("Подключено · интернет есть", "Connected · online").to_string(), GOOD),
                    Link::NoInternet(_) => (t!("Подключено, но интернета нет", "Connected, no internet").into(), WARN),
                    Link::Cable if eth_n() > 0 => (t!("Подключено по Ethernet · интернет есть", "Connected via Ethernet · online").into(), GOOD),
                    Link::Cable => (t!("Интернет есть", "Online").into(), GOOD), // not Wi-Fi and no cable we can name (a phone's USB link, a virtual adapter)
                    Link::CableNoInternet => (t!("Ethernet подключён, но интернета нет", "Ethernet connected, no internet").into(), WARN),
                    Link::Offline if !wifi => (t!("Сетевой кабель не подключён", "Network cable unplugged").into(), pal().dim),
                    Link::Offline if on => (t!("Не подключено", "Not connected").into(), pal().dim),
                    Link::Offline => (t!("Выключен", "Turned off").into(), pal().dim),
                }
            };
            SelectObject(dc, f_small);
            text(dc, &sub, r(64, 40, w - 90, 60), sub_col, DT_LEFT);
            // on/off switch: the knob glides, the track fades to the accent
            if wifi {
                let sw = r(w - 66, 24, w - 18, 48);
                let ts = ease(hv(1_500_000, on));
                fill(dc, sw, mix(mix(pal().line, pal().dim, 0.25), pal().accent, ts), sc(24));
                let kx = sw.left + sc(12) + ((sw.right - sw.left - sc(24)) as f32 * ts) as i32;
                fill(dc, RECT { left: kx - sc(8), top: sw.top + sc(4), right: kx + sc(8), bottom: sw.bottom - sc(4) }, mix(pal().text, pal().bg, ts), sc(16));
            }
            fill(dc, r(16, 68, w - 16, 69), pal().line, 0);
            let rs = rects(k);
            let pass = PASS.with_borrow(|p| p.clone());
            if let Some((i, p)) = pass {
                let name = NETS.with_borrow(|n| n.get(i).map(|n| n.ssid.clone()).unwrap_or_default());
                SelectObject(dc, f_txt);
                text(dc, &tf!("Пароль для «{name}»", "Password for “{name}”"), r(18, 82, w - 18, 106), pal().text, DT_LEFT);
                fill(dc, r(16, 112, w - 16, 150), pal().accent, sc(8));
                fill(dc, r(17, 113, w - 17, 149), mix(pal().bg, 0, 0.35), sc(8));
                let dots = "•".repeat(p.chars().count());
                text(dc, &dots, r(30, 112, w - 30, 150), pal().text, DT_LEFT);
                // a blinking caret after the dots
                if now_ms() / 530 % 2 == 0 {
                    let mut tr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                    let wd = wide(&dots);
                    DrawTextW(dc, wd.as_ptr(), -1, &mut tr, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
                    let cx = sc(30) + tr.right - tr.left + sc(2);
                    fill(dc, RECT { left: cx, top: sc(122), right: cx + sc(2), bottom: sc(140) }, pal().accent, 0);
                }
                BUSY.set(true);
                SelectObject(dc, f_small);
                text(dc, t!("Enter — подключиться · Esc — отмена", "Enter — connect · Esc — cancel"), r(18, 158, w - 18, 180), pal().dim, DT_LEFT);
            } else {
                let (_, label, _) = wifi_rows();
                let conn = CONN.with_borrow(|c| c.as_ref().map(|c| (c.q, c.rx, c.tx, c.phy)));
                let age = now_ms().saturating_sub(T0.get()) as f32;
                // wired connections first: a card each with the link's speed, the address and the adapter
                ETHS.with_borrow(|eths| {
                    for (i, e) in eths.iter().take(2).enumerate() {
                        let top = 78 + ETH_CARD * i as i32;
                        let rc = r(12, top, w - 12, top + ETH_CARD - 8);
                        let card = mix(pal().bg, pal().active, 0.85);
                        fill(dc, rc, card, sc(12));
                        let online = INET.get();
                        eth_glyph(dc, rc.left + sc(30), rc.top + sc(32), sc(20), f_glyph, pal().accent, pal().text, if online { 0 } else { 2 });
                        SelectObject(dc, f_title);
                        text(dc, &e.name, RECT { left: rc.left + sc(60), top: rc.top + sc(12), right: rc.right - sc(14), bottom: rc.top + sc(36) }, pal().text, DT_LEFT);
                        let (dot, words) = if online { (GOOD, t!("Подключено · интернет есть", "Connected · online")) } else { (WARN, t!("Подключено · нет интернета", "Connected · no internet")) };
                        let dy = rc.top + sc(46);
                        fill(dc, RECT { left: rc.left + sc(60), top: dy - sc(3), right: rc.left + sc(66), bottom: dy + sc(3) }, dot, sc(6));
                        SelectObject(dc, f_small);
                        text(dc, words, RECT { left: rc.left + sc(72), top: dy - sc(10), right: rc.right - sc(14), bottom: dy + sc(10) }, dot, DT_LEFT);
                        let det = [net::speed_text(e.bps), e.ip.clone(), e.desc.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                        text(dc, &det, RECT { left: rc.left + sc(60), top: rc.top + sc(58), right: rc.right - sc(14), bottom: rc.top + sc(76) }, pal().dim, DT_LEFT);
                    }
                });
                NETS.with_borrow(|nets| {
                    if nets.is_empty() && (wifi || eth_n() == 0) {
                        SelectObject(dc, f_txt);
                        let words = if !wifi { t!("Подключите сетевой кабель", "Plug in a network cable") } else if on { t!("Рядом нет сетей", "No networks nearby") } else { t!("Включите Wi‑Fi, чтобы увидеть сети", "Turn Wi‑Fi on to see networks") };
                        let top = 86 + ETH_CARD * eth_n();
                        text(dc, words, r(18, top, w - 18, top + 44), pal().dim, DT_LEFT);
                    }
                    if let Some(ly) = label {
                        SelectObject(dc, f_small);
                        text(dc, t!("Другие сети", "Other networks"), r(20, ly + 6, w - 16, ly + 26), pal().dim, DT_LEFT);
                    }
                    for (i, n) in nets.iter().enumerate() {
                        let rc = rs[i].0;
                        // rows fade in one after another when the panel opens
                        let t = ease(((age - 45.0 * i as f32) / 240.0).clamp(0.0, 1.0));
                        if t < 1.0 {
                            BUSY.set(true);
                        }
                        let ink = |c: u32| mix(pal().bg, c, t);
                        let lit = arcs_lit(n.q);
                        if n.connected {
                            let card = mix(pal().bg, pal().active, 0.85);
                            fill(dc, rc, mix(pal().bg, card, t), sc(12));
                            let online = !matches!(state, Link::NoInternet(_));
                            wifi_glyph(dc, rc.left + sc(30), rc.top + sc(32), sc(26), lit, ink(pal().accent), ink(mix(pal().dim, card, 0.45)), if online { 0 } else { 2 });
                            SelectObject(dc, f_title);
                            text(dc, &n.ssid, RECT { left: rc.left + sc(60), top: rc.top + sc(12), right: rc.right - sc(14), bottom: rc.top + sc(36) }, ink(pal().text), DT_LEFT);
                            let (dot, words) = if online { (GOOD, t!("Интернет есть", "Online")) } else { (WARN, t!("Нет интернета", "No internet")) };
                            let dy = rc.top + sc(46);
                            fill(dc, RECT { left: rc.left + sc(60), top: dy - sc(3), right: rc.left + sc(66), bottom: dy + sc(3) }, ink(dot), sc(6));
                            SelectObject(dc, f_small);
                            text(dc, words, RECT { left: rc.left + sc(72), top: dy - sc(10), right: rc.right - sc(14), bottom: dy + sc(10) }, ink(dot), DT_LEFT);
                            let q = conn.map(|c| c.0).unwrap_or(n.q);
                            let det = match conn {
                                Some((_, rx, tx, phy)) => format!("{phy} · {q}% · ↓{rx} ↑{tx} {} · {}", t!("Мбит/с", "Mbps"), auth_name(n.auth)),
                                None => format!("{q}% · {}", auth_name(n.auth)),
                            };
                            text(dc, &det, RECT { left: rc.left + sc(60), top: rc.top + sc(58), right: rc.right - sc(14), bottom: rc.top + sc(76) }, ink(pal().dim), DT_LEFT);
                            // a quiet Disconnect button that turns red under the pointer
                            if let Some(&(ob, _)) = rs.iter().find(|(_, a)| *a == Act::NetOff) {
                                g(Act::NetOff, ob, mix(card, pal().bg, 0.5), mix(card, BAD, 0.3), sc(8));
                                SelectObject(dc, f_small);
                                text(dc, t!("Отключиться", "Disconnect"), ob, ink(if hot(Act::NetOff) { pal().text } else { mix(pal().dim, pal().text, 0.5) }), DT_CENTER);
                            }
                        } else {
                            g(Act::Net(i), rc, pal().bg, pal().hover, sc(10));
                            wifi_glyph(dc, rc.left + sc(22), rc.top + sc(24), sc(16), lit, ink(pal().text), ink(mix(pal().dim, pal().bg, 0.5)), 0);
                            let saved = n.profile.is_some();
                            let sub = if !n.secure { t!("Открытая сеть", "Open network") } else if saved { t!("Сохранена", "Saved") } else { "" };
                            SelectObject(dc, f_txt);
                            let name_rc = if sub.is_empty() { RECT { left: rc.left + sc(46), right: rc.right - sc(110), ..rc } } else { RECT { left: rc.left + sc(46), top: rc.top + sc(5), right: rc.right - sc(110), bottom: rc.top + sc(27) } };
                            text(dc, &n.ssid, name_rc, ink(pal().text), DT_LEFT);
                            if !sub.is_empty() {
                                SelectObject(dc, f_small);
                                text(dc, sub, RECT { left: rc.left + sc(46), top: rc.top + sc(25), right: rc.right - sc(110), bottom: rc.bottom - sc(5) }, ink(if n.secure { pal().dim } else { WARN }), DT_LEFT);
                            }
                            if hot(Act::Net(i)) {
                                SelectObject(dc, f_small);
                                let action = if saved || !n.secure { t!("Подключить", "Connect") } else { t!("Ввести пароль", "Enter password") };
                                text(dc, action, RECT { left: rc.right - sc(120), right: rc.right - sc(14), ..rc }, pal().accent, DT_RIGHT);
                            } else if n.secure {
                                SelectObject(dc, f_glyph);
                                text(dc, "\u{E72E}", RECT { left: rc.right - sc(40), right: rc.right - sc(8), ..rc }, ink(mix(pal().dim, pal().bg, 0.35)), DT_CENTER); // lock
                            }
                        }
                    }
                });
            }
            let last = rs.last().unwrap();
            g(last.1, last.0, pal().bg, pal().hover, sc(8));
            SelectObject(dc, f_txt);
            text(dc, t!("Параметры сети", "Network settings"), RECT { left: last.0.left + sc(10), ..last.0 }, pal().accent, DT_LEFT);
            SelectObject(dc, f_glyph);
            text(dc, "\u{E76C}", RECT { left: last.0.right - sc(36), ..last.0 }, pal().accent, DT_CENTER); // chevron right
        }
        Kind::Menu => {
            // header: the app's icon on an accent-tinted tile, its name, and what is open
            MHEAD.with_borrow(|hd| {
                if let Some((ic, glyph, title, sub)) = hd {
                    let tile = r(14, 12, 48, 46);
                    fill(dc, tile, mix(pal().bg, pal().accent, 0.16), sc(10));
                    if !ic.is_null() {
                        DrawIconEx(dc, tile.left + sc(5), tile.top + sc(5), *ic, sc(24), sc(24), 0, null_mut(), DI_NORMAL);
                    } else {
                        SelectObject(dc, f_glyph); // no app icon (Style, Language): the menu's own symbol
                        text(dc, glyph, tile, pal().accent, DT_CENTER);
                    }
                    SelectObject(dc, f_title);
                    text(dc, title, r(60, 10, w - 14, 32), pal().text, DT_LEFT);
                    SelectObject(dc, f_small);
                    text(dc, sub, r(60, 30, w - 14, 48), pal().dim, DT_LEFT);
                    fill(dc, r(12, MHEAD_H + 4, w - 12, MHEAD_H + 5), pal().line, 0);
                }
            });
            let rs = rects(k);
            MITEMS.with_borrow(|items| {
                for ((rc, a), it) in rs.iter().zip(items) {
                    if it.sep {
                        fill(dc, RECT { left: rc.left + sc(6), top: rc.top - sc(5), right: rc.right - sc(6), bottom: rc.top - sc(5) + 1 }, pal().line, 0);
                    }
                    let col = if it.danger { BAD } else { pal().text };
                    g(*a, *rc, pal().bg, if it.danger { mix(pal().bg, BAD, 0.22) } else { pal().hover }, sc(8));
                    // plain text rows: a window's own icon is the only picture; the current choice gets an accent dot
                    let mut lx = rc.left + sc(14);
                    if !it.icon.is_null() {
                        DrawIconEx(dc, lx, (rc.top + rc.bottom) / 2 - sc(8), it.icon, sc(16), sc(16), 0, null_mut(), DI_NORMAL);
                        lx += sc(26);
                    }
                    let mut rx = rc.right - sc(14);
                    if it.on {
                        let (cy, d) = ((rc.top + rc.bottom) / 2, sc(4));
                        fill(dc, RECT { left: rx - 2 * d, top: cy - d, right: rx, bottom: cy + d }, pal().accent, 2 * d);
                        rx -= sc(18);
                    }
                    SelectObject(dc, f_small);
                    if !it.hint.is_empty() {
                        text(dc, &it.hint, RECT { left: lx, right: rx, ..*rc }, mix(pal().dim, pal().bg, 0.25), DT_RIGHT);
                    }
                    SelectObject(dc, f_txt);
                    text(dc, &it.label, RECT { left: lx, right: rx - if it.hint.is_empty() { 0 } else { sc(96) }, ..*rc }, if it.on { pal().accent } else { col }, DT_LEFT);
                }
            });
        }
        Kind::Start => {
            fill(dc, r(20, 20, w - 20, 56), mix(pal().bg, 0, 0.35), sc(18));
            SelectObject(dc, f_glyph);
            text(dc, "\u{E721}", r(28, 20, 56, 56), pal().dim, DT_CENTER);
            SelectObject(dc, f_txt);
            let q = QUERY.with_borrow(|q| q.clone());
            if q.is_empty() {
                text(dc, "Type to search apps", r(64, 20, w - 28, 56), pal().dim, DT_LEFT);
            } else {
                text(dc, &q, r(64, 20, w - 28, 56), pal().text, DT_LEFT);
            }
            let f = filtered();
            if f.is_empty() {
                if q.is_empty() {
                    text(dc, "Nothing here yet", r(24, 76, w - 24, 106), pal().text, DT_LEFT);
                    text(dc, "Drag an app or file onto the Start button to add it", r(24, 106, w - 24, 134), pal().dim, DT_LEFT);
                    text(dc, "Or search an app and right-click it → Add to Start", r(24, 134, w - 24, 162), pal().dim, DT_LEFT);
                    text(dc, t!("Или правый клик по панели → Меню «Пуск» → импорт из Windows", "Or right-click the bar → Start menu → import what Windows has pinned"), r(24, 162, w - 24, 190), pal().dim, DT_LEFT);
                } else {
                    text(dc, "No apps found", r(24, 76, w - 24, 116), pal().dim, DT_LEFT);
                }
            }
            let rs = rects(k);
            let apps = APPS.lock().unwrap();
            for (rc, a) in &rs {
                match *a {
                    Act::App(i) => {
                        // the list is reloaded on a background thread: an index from a moment ago may be gone
                        let Some(app) = f.get(i).and_then(|&ai| apps.get(ai)) else { continue };
                        // the row that was just pinned pulses in the accent colour for a moment
                        let pulse = JUST.with_borrow(|j| {
                            let age = now_ms().saturating_sub(j.1) as f32 / 1200.0;
                            if age < 1.0 && !j.0.is_empty() && app.1 == wide(&j.0) { (1.0 - age) * (1.0 - age) } else { 0.0 }
                        });
                        let sel = i == SEL.get(); // what Enter opens
                        if pulse > 0.0 || sel {
                            BUSY.set(BUSY.get() || pulse > 0.0);
                            fill(dc, *rc, mix(if sel { mix(pal().bg, pal().accent, 0.14) } else { pal().bg }, pal().accent, 0.45 * pulse), sc(8));
                        }
                        g(*a, *rc, if sel { mix(pal().bg, pal().accent, 0.14) } else { pal().bg }, if sel { mix(pal().hover, pal().accent, 0.18) } else { pal().hover }, sc(8));
                        if sel {
                            fill(dc, RECT { left: rc.left, top: rc.top + sc(10), right: rc.left + sc(3), bottom: rc.bottom - sc(10) }, pal().accent, sc(3));
                        }
                        let ic = ICONS.with_borrow_mut(|m| {
                            *m.entry(app.1.clone()).or_insert_with(|| {
                                let t = lnk_target(&app.1);
                                lnk_icon(&app.1, &t)
                            })
                        });
                        if !ic.is_null() {
                            DrawIconEx(dc, rc.left + sc(13), (rc.top + rc.bottom - icon_px()) / 2, ic, icon_px(), icon_px(), 0, null_mut(), DI_NORMAL); // at its loaded size: no stretching
                        }
                        let store = wstr(&app.1).starts_with("shell:");
                        text(dc, &app.0, RECT { left: rc.left + sc(52), top: rc.top, right: rc.right - sc(110), bottom: rc.bottom }, pal().text, DT_LEFT);
                        SelectObject(dc, f_small);
                        let kind = if sel && !q.is_empty() { "Enter ↵" } else if store { t!("Приложение", "App") } else { "" };
                        text(dc, kind, RECT { left: rc.right - sc(110), top: rc.top, right: rc.right - sc(14), bottom: rc.bottom }, pal().dim, DT_RIGHT);
                        SelectObject(dc, f_txt);
                    }
                    Act::Power(p) => {
                        let armed = CONFIRM.get() == p as i32;
                        glow(dc, rc.top * 4096 + rc.left + 1_000_000, armed || hot(*a), *rc, pal().bg, if armed { BAD } else { pal().hover }, sc(8));
                        SelectObject(dc, f_glyph);
                        text(dc, POWER_GLYPH[(p as usize).min(6)], *rc, pal().text, DT_CENTER);
                    }
                    _ => {}
                }
            }
            SelectObject(dc, f_small);
            let note = NOTE.with_borrow(|n| n.clone());
            let hint = match CONFIRM.get() {
                1 => t!("Нажмите ещё раз, чтобы выйти из системы", "Click again to sign out").to_string(),
                4 => t!("Нажмите ещё раз для перезагрузки", "Click again to restart").to_string(),
                5 => t!("Нажмите ещё раз для выключения", "Click again to shut down").to_string(),
                _ if !note.is_empty() => note,
                _ => std::env::var("USERNAME").unwrap_or_default(),
            };
            text(dc, &hint, r(24, h - 52, w - 24 - 44 * power_shown().len() as i32, h - 12), pal().dim, DT_LEFT);
        }
    }

    BitBlt(wdc, 0, 0, cr.right, cr.bottom, dc, 0, 0, SRCCOPY);
    SelectObject(dc, of);
    for f in [f_txt, f_small, f_glyph, f_title, f_big] {
        DeleteObject(f);
    }
    SelectObject(dc, old);
    DeleteObject(bmp);
    DeleteDC(dc);
    if BUSY.get() {
        SetTimer(hw, 2, 16, None); // next animation frame
    } else {
        KillTimer(hw, 2);
    }
    EndPaint(hw, &ps);
}

// ---------- window proc ----------

unsafe extern "system" fn proc(hw: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_TIMER && wp == 3 {
        anim_step(hw); // also for the fading-out ghost window
        return 0;
    }
    if hw != FH.get() {
        return DefWindowProcW(hw, msg, wp, lp); // ghost or a window still being created
    }
    let Some(k) = FLY.get() else { return DefWindowProcW(hw, msg, wp, lp) };
    let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16 & 0xffff) as i16 as i32);
    match msg {
        WM_PAINT => paint(hw, k),
        WM_ERASEBKGND => return 1,
        WM_ACTIVATE if wp & 0xffff == 0 => {
            // A file drag that began elsewhere (desktop icon, folder window) deactivates this panel with the button still
            // down: keep Start open so the drop can land on it, and close once the button is up (timer 6).
            if k == Kind::Start && GetAsyncKeyState(VK_LBUTTON as i32) < 0 {
                SetTimer(hw, 6, 150, None);
            } else {
                close();
            }
        }
        WM_TIMER if wp == 6 => {
            if GetAsyncKeyState(VK_LBUTTON as i32) >= 0 {
                KillTimer(hw, 6);
                if GetForegroundWindow() != hw {
                    close();
                }
            }
        }
        WM_DROPFILES => on_files(hw, wp as HDROP),
        WM_TIMER if wp == 2 => {
            InvalidateRect(hw, null(), 0);
        }
        WM_TIMER if wp == 7 => {
            // the pointer rested on one of Start's power buttons: say what it does
            KillTimer(hw, 7);
            if let Some((rc, Act::Power(p))) = OVER.get().and_then(|o| rects(k).into_iter().find(|x| x.1 == o)) {
                let (x, y) = FIN.get();
                show_tip_at(format!("{}{}", power_label(p), if p == 0 { " (Win+L)" } else { "" }), x + (rc.left + rc.right) / 2, Some(y + rc.top));
            }
        }
        WM_MOUSELEAVE => {
            KillTimer(hw, 7);
            ShowWindow(TIPWND.get(), SW_HIDE);
            if OVER.replace(None).is_some() {
                InvalidateRect(hw, null(), 0); // no highlight stays behind under a pointer that left
            }
        }
        WM_TIMER => {
            if (k == Kind::Wifi && PASS.with_borrow(|p| p.is_none())) || (k == Kind::Sound && DRAG.get() == 0) {
                // what is listed may have changed (a network appeared, a headset was plugged in): the panel follows
                if k == Kind::Wifi {
                    read_nets();
                    read_wifi();
                } else {
                    read_sound();
                }
                let (w, h) = (sc(size(k).0), sc(size(k).1));
                // the panel hangs above the bar: keep its bottom edge where it is and grow / shrink upward
                let (x, _) = FIN.get();
                let y = pop_y(h);
                FIN.set((x, y));
                SetWindowPos(hw, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                SetWindowRgn(hw, CreateRoundRectRgn(0, 0, w + 1, h + 1, rr(16), rr(16)), 1);
            }
            InvalidateRect(hw, null(), 0);
        }
        WM_MOUSEMOVE => {
            if DRAG.get() != 0 {
                let want = match DRAG.get() {
                    1 => Act::VolSlide,
                    2 => Act::BrightSlide,
                    _ => Act::MicSlide,
                };
                if let Some((rc, a)) = rects(k).into_iter().find(|(_, a)| *a == want) {
                    slide(a, x, rc);
                }
            }
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hw, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            let o = act_at(k, x, y).map(|a| a.1);
            let was = OVER.replace(o);
            if was != o {
                // Start's power buttons are bare icons: a tooltip names the one the pointer rests on
                ShowWindow(TIPWND.get(), SW_HIDE);
                if matches!(o, Some(Act::Power(_))) { SetTimer(hw, 7, 450, None) } else { KillTimer(hw, 7) as usize };
            }
            if was != o || DRAG.get() != 0 {
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_LBUTTONDOWN => {
            KillTimer(hw, 7);
            ShowWindow(TIPWND.get(), SW_HIDE);
            if let Some((rc, a)) = act_at(k, x, y) {
                run_act(a, rc, x);
            }
            if FLY.get().is_some() {
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_RBUTTONUP => {
            if let (Kind::Start, Some((_, Act::App(i)))) = (k, act_at(k, x, y)) {
                app_menu(hw, i, x, y);
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_LBUTTONUP => {
            let d = DRAG.replace(0);
            if d != 0 {
                ReleaseCapture();
                if d == 2 {
                    set_brightness(BRIGHT.load(Relaxed));
                }
            }
        }
        WM_MOUSEWHEEL => {
            let up = (wp >> 16) as i16 > 0;
            match k {
                Kind::Quick | Kind::Sound => add_volume(if up { 2 } else { -2 }),
                Kind::Start => SCROLL.set(if up { SCROLL.get().saturating_sub(2) } else { (SCROLL.get() + 2).min(filtered().len().saturating_sub(ROWS)) }),
                _ => {}
            }
            InvalidateRect(hw, null(), 0);
        }
        WM_KEYDOWN if k == Kind::Start && (wp == VK_DOWN as usize || wp == VK_UP as usize) => {
            // arrows move the row Enter opens; the list scrolls to keep it in view
            let n = filtered().len();
            if n > 0 {
                let s = if wp == VK_DOWN as usize { (SEL.get() + 1).min(n - 1) } else { SEL.get().saturating_sub(1) };
                SEL.set(s);
                SCROLL.set(SCROLL.get().min(s).max((s + 1).saturating_sub(ROWS)));
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_CHAR => {
            let c = wp as u32;
            if c == 27 {
                if PASS.with_borrow(|p| p.is_some()) {
                    PASS.set(None);
                } else if k == Kind::Start && QUERY.with_borrow(|q| !q.is_empty()) {
                    QUERY.set(String::new()); // Esc clears the search first, the next one closes
                    SCROLL.set(0);
                    SEL.set(0);
                } else {
                    close();
                    return 0;
                }
            } else if k == Kind::Wifi && PASS.with_borrow(|p| p.is_some()) {
                if c == 13 {
                    if let Some((i, p)) = PASS.take() {
                        NETS.with_borrow(|n| wifi_join(&n[i], &p));
                    }
                } else if let Some(p) = PASS.with_borrow_mut(|p| p.as_mut().map(|p| &mut p.1 as *mut String)) {
                    let p = &mut *p;
                    if c == 8 {
                        p.pop();
                    } else if let Some(ch) = char::from_u32(c).filter(|_| c >= 32) {
                        p.push(ch);
                    }
                }
            } else if k == Kind::Start {
                if c == 13 {
                    if launch(SEL.get()) {
                        close();
                        return 0;
                    }
                } else {
                    QUERY.with_borrow_mut(|q| {
                        if c == 8 {
                            q.pop();
                        } else if let Some(ch) = char::from_u32(c).filter(|_| c >= 32) {
                            q.push(ch);
                        }
                    });
                    SCROLL.set(0);
                    SEL.set(0);
                    find_loose_exes(hw);
                }
            }
            InvalidateRect(hw, null(), 0);
        }
        _ => return DefWindowProcW(hw, msg, wp, lp),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{flip_layout, layout_entries, power_mask, power_names, score};

    #[test]
    fn start_buttons_from_the_config_list() {
        assert_eq!(power_mask("lock,sleep,restart,shutdown"), 0b110101);
        assert_eq!(power_mask(" Lock , SHUTDOWN, nonsense"), 0b100001);
        assert_eq!(power_mask(""), 0);
        assert_eq!(power_names(0b110101), "lock,sleep,restart,shutdown");
        assert_eq!(power_mask(&power_names(0b111111)), 0b111111);
    }

    #[test]
    fn windows_start_layout_in_both_formats() {
        let xml = r#"<start:Group Name="Work"><start:DesktopApplicationTile Size="2x2" Column="0" Row="0" DesktopApplicationLinkPath="%APPDATA%\Microsoft\Windows\Start Menu\Programs\A &amp; B.lnk" /><start:Tile Size="2x2" AppUserModelID="Microsoft.WindowsCalculator_8wekyb3d8bbwe!App" /><start:DesktopApplicationTile DesktopApplicationID="Microsoft.Windows.Explorer" /></start:Group>"#;
        assert_eq!(layout_entries(xml), [r"%APPDATA%\Microsoft\Windows\Start Menu\Programs\A & B.lnk", r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", r"shell:AppsFolder\Microsoft.Windows.Explorer"]);
        let json = r#"{"pinnedList":[{"packagedAppId":"Microsoft.WindowsStore_8wekyb3d8bbwe!App"},{"desktopAppLink": "%ALLUSERSPROFILE%\\Microsoft\\Windows\\Start Menu\\Programs\\Word.lnk"},{"desktopAppId":"MSEdge"}]}"#;
        assert_eq!(layout_entries(json), [r"shell:AppsFolder\Microsoft.WindowsStore_8wekyb3d8bbwe!App", r"%ALLUSERSPROFILE%\Microsoft\Windows\Start Menu\Programs\Word.lnk", r"shell:AppsFolder\MSEdge"]);
        assert!(layout_entries("<LayoutModificationTemplate />").is_empty());
    }

    #[test]
    fn layout_flip_both_ways() {
        assert_eq!(flip_layout("сркщьу"), "chrome");
        assert_eq!(flip_layout("ntktuhfv"), "телеграм");
        assert_eq!(flip_layout("vs code"), "мы сщву");
    }

    #[test]
    fn ranking() {
        assert_eq!(score("visual studio code", "vis"), Some(0));
        assert_eq!(score("visual studio code", "cod"), Some(1));
        assert_eq!(score("visual studio code", "vsc"), Some(2));
        assert_eq!(score("calculator", "cul"), Some(3));
        assert_eq!(score("calculator", "xyz"), None);
        assert_eq!(score("calculator", ""), None);
        assert_eq!(score("v", "v"), Some(0)); // one-letter initials do not count, the prefix does
    }
}
