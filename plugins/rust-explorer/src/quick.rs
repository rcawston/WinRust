//! Our own Quick access (the first section of the left pane) and the topbar's Start list. Both are plain text files, one
//! path per line (UTF-8), edited from the right-click menus (menu.rs) or by hand. Quick access starts as a copy of the
//! folders pinned in Windows' Quick access and is ours from then on: pinning here never touches Explorer's list.
use std::{env, fs, path::{Path, PathBuf}, sync::atomic::{AtomicU32, Ordering::Relaxed}};
use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, PostMessageW, RegisterWindowMessageW};

fn appdata(rel: &str) -> PathBuf { PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join(rel) }

pub fn file() -> PathBuf { appdata(r"rust-explorer\quick-access.txt") }
/// The topbar's Start list (paths of .lnk / .exe files): it reads the file again every time Start opens.
fn start_file() -> PathBuf { appdata("topbar-start.txt") }

// ---------- the list itself: pure, see the tests ----------

/// How two paths are compared: any case, with or without the closing backslash ("C:\" stays a root).
fn key(p: &str) -> String {
    let t = p.trim().trim_end_matches(['\\', '/']);
    if t.ends_with(':') { format!("{t}\\").to_lowercase() } else { t.to_lowercase() }
}

/// One path per line; blank lines, the spaces around a path and repeats of a path are dropped.
pub fn parse(text: &str) -> Vec<String> {
    let mut out = vec![];
    for l in text.trim_start_matches('\u{feff}').lines() { add(&mut out, l); }
    out
}

pub fn text(list: &[String]) -> String { list.join("\n") }

pub fn find(list: &[String], p: &str) -> Option<usize> {
    let k = key(p);
    list.iter().position(|x| key(x) == k)
}

/// Appends `p` unless it is there already. true = the list changed (the same for remove and shift).
pub fn add(list: &mut Vec<String>, p: &str) -> bool {
    let p = p.trim();
    if p.is_empty() || find(list, p).is_some() { return false; }
    list.push(p.to_string());
    true
}

pub fn remove(list: &mut Vec<String>, p: &str) -> bool { find(list, p).map(|i| list.remove(i)).is_some() }

/// Moves `p` one place up or down.
pub fn shift(list: &mut [String], p: &str, up: bool) -> bool {
    let Some(i) = find(list, p) else { return false };
    let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|&j| j < list.len()) };
    j.map(|j| list.swap(i, j)).is_some()
}

/// Title and glyph (Segoe MDL2 Assets) of a pinned folder in the pane: the user's well-known folders get their own,
/// a drive its letter, anything else the folder's name.
pub fn look(path: &str, home: &str) -> (String, &'static str) {
    const FOLDER: &str = "\u{E8B7}";
    if !home.is_empty() && key(path) == key(home) { return (t!("Главная", "Home").into(), "\u{E80F}"); }
    let p = Path::new(path);
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else { return (path.trim().to_string(), if path.contains(':') { "\u{EDA2}" } else { FOLDER }) };
    let (title, glyph) = match name.to_lowercase().as_str() {
        "desktop" => (t!("Рабочий стол", "Desktop"), "\u{E7F4}"),
        "downloads" => (t!("Загрузки", "Downloads"), "\u{E896}"),
        "documents" => (t!("Документы", "Documents"), "\u{E8A5}"),
        "pictures" => (t!("Изображения", "Pictures"), "\u{E91B}"),
        "music" => (t!("Музыка", "Music"), "\u{E8D6}"),
        "videos" => (t!("Видео", "Videos"), "\u{E714}"),
        _ => return (name, FOLDER),
    };
    (title.into(), glyph)
}

/// Start takes programs and shortcuts only.
pub fn startable(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("lnk"))
}

// ---------- the files ----------

/// None = there is no such file (yet). Bytes that are not UTF-8 do not lose the rest of the list.
fn read(f: &Path) -> Option<Vec<String>> { fs::read(f).ok().map(|b| parse(&String::from_utf8_lossy(&b))) }

fn write(f: &Path, list: &[String]) {
    let _ = fs::create_dir_all(f.parent().unwrap_or(f));
    let _ = fs::write(f, text(list));
}

/// What the pane showed before the list was ours: the user's folder and its usual subfolders.
fn defaults() -> Vec<String> {
    let home = PathBuf::from(env::var_os("USERPROFILE").unwrap_or_default());
    let subs = ["Desktop", "Downloads", "Documents", "Pictures", "Music", "Videos"].iter().map(|s| home.join(s)).filter(|p| p.is_dir());
    [home.clone()].into_iter().chain(subs).map(|p| p.to_string_lossy().into_owned()).collect()
}

/// Our Quick access list. The first time (no file yet) it is what is pinned in Windows' Quick access, or the usual
/// user folders when that gives nothing; an emptied list stays empty.
pub fn load() -> Vec<String> {
    read(&file()).unwrap_or_else(|| {
        let mut v = parse(&crate::sh::os_quick_access().join("\n"));
        if v.is_empty() { v = defaults(); }
        write(&file(), &v);
        v
    })
}

pub fn pinned(p: &str) -> bool { find(&load(), p).is_some() }

/// Changes the list on disk (read again first: another window may have changed it meanwhile) and tells every file
/// manager window to show it (`msg`).
pub fn edit(change: impl FnOnce(&mut Vec<String>) -> bool) {
    let mut v = load();
    if change(&mut v) {
        write(&file(), &v);
        unsafe { PostMessageW(HWND_BROADCAST, msg(), 0, 0) };
    }
}

/// The broadcast a window gets when the Quick access list changed: fill the pane again.
pub fn msg() -> u32 {
    static M: AtomicU32 = AtomicU32::new(0);
    match M.load(Relaxed) {
        0 => {
            let n: Vec<u16> = "RustExplorerQuickAccess\0".encode_utf16().collect();
            let m = unsafe { RegisterWindowMessageW(n.as_ptr()) };
            M.store(m, Relaxed);
            m
        }
        m => m,
    }
}

/// All of `paths` are in the topbar's Start.
pub fn in_start(paths: &[String]) -> bool {
    let v = read(&start_file()).unwrap_or_default();
    !paths.is_empty() && paths.iter().all(|p| find(&v, p).is_some())
}

/// Adds `paths` to the topbar's Start (at the end, in order) or takes them out.
pub fn start_set(paths: &[String], on: bool) {
    let mut v = read(&start_file()).unwrap_or_default();
    let changed = paths.iter().fold(false, |ch, p| (if on { add(&mut v, p) } else { remove(&mut v, p) }) | ch);
    if changed { write(&start_file(), &v); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn a_file_is_one_path_per_line_without_repeats() {
        let v = parse("\u{feff}C:\\src\r\n\r\n  D:\\Games \r\nc:\\SRC\\\r\nC:\\Users\\me\\Desktop\n");
        assert_eq!(v, list(&["C:\\src", "D:\\Games", "C:\\Users\\me\\Desktop"])); // BOM, blank line, spaces, the same folder twice
        assert_eq!(parse(&text(&v)), v); // what is written reads back the same
        assert_eq!(text(&v), "C:\\src\nD:\\Games\nC:\\Users\\me\\Desktop");
        assert!(parse("").is_empty());
    }

    #[test]
    fn paths_match_in_any_case_and_with_or_without_the_last_backslash() {
        let v = list(&["C:\\Src", "D:\\"]);
        assert_eq!(find(&v, "c:\\src\\"), Some(0));
        assert_eq!(find(&v, "d:"), Some(1)); // a drive is its root
        assert_eq!(find(&v, "D:\\"), Some(1));
        assert_eq!(find(&v, "C:\\Src2"), None);
        assert_eq!(find(&list(&["C:\\Папка"]), "c:\\папка"), Some(0));
    }

    #[test]
    fn pin_unpin_and_reorder() {
        let mut v = list(&["A:\\a", "B:\\b"]);
        assert!(add(&mut v, "C:\\c") && !add(&mut v, "c:\\C\\") && !add(&mut v, "  "));
        assert_eq!(v, list(&["A:\\a", "B:\\b", "C:\\c"])); // new ones go to the end, once
        assert!(shift(&mut v, "c:\\c", true));
        assert_eq!(v, list(&["A:\\a", "C:\\c", "B:\\b"]));
        assert!(shift(&mut v, "A:\\a", false) && !shift(&mut v, "C:\\c", true)); // the first one cannot go higher
        assert_eq!(v, list(&["C:\\c", "A:\\a", "B:\\b"]));
        assert!(!shift(&mut v, "B:\\b", false) && !shift(&mut v, "X:\\none", true)); // nor the last one lower
        assert!(remove(&mut v, "a:\\A") && !remove(&mut v, "A:\\a"));
        assert_eq!(v, list(&["C:\\c", "B:\\b"]));
    }

    #[test]
    fn pinned_folders_get_a_title_and_a_glyph() {
        let home = "C:\\Users\\me";
        assert_eq!(look("c:\\users\\ME\\", home), ("Главная".to_string(), "\u{E80F}"));
        assert_eq!(look("C:\\Users\\me\\Downloads", home), ("Загрузки".to_string(), "\u{E896}"));
        assert_eq!(look("D:\\OneDrive\\Desktop", home).1, "\u{E7F4}"); // a redirected Desktop is still the Desktop
        assert_eq!(look("C:\\src", home), ("src".to_string(), "\u{E8B7}"));
        assert_eq!(look("D:\\", home), ("D:\\".to_string(), "\u{EDA2}"));
    }

    #[test]
    fn start_takes_programs_and_shortcuts() {
        assert!(startable(Path::new("C:\\a\\App.EXE")) && startable(Path::new("C:\\a\\App.lnk")));
        assert!(!startable(Path::new("C:\\a\\notes.txt")) && !startable(Path::new("C:\\a\\folder")));
    }
}
