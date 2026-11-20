//! Our own right-click menus: the selection, the folder background, the left pane and the tab strip. Plain popup menus
//! (dark through ui::dark_mode). "Pin to Quick access" and "Add to Start" act on our lists (quick.rs), not on Explorer's.
//! The stock shell menu with its extensions (7-Zip, Git, Open with) is one click away behind "More options…".
//! shortcuts: no icons, no submenus ("New" is two entries), no keyboard accelerators in the labels.
#![allow(unsafe_op_in_unsafe_fn)]
use super::*;
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cmd {
    Sep,
    Open, NewTab, NewWindow, RunAs, Cut, Copy, Paste, CopyPath, Rename, Delete,
    Pin, Unpin, PinHere, UnpinHere, Up, Down, AddStart, RemoveStart, Props, More,
    NewFolder, NewFile, Refresh, Disk, Tabs, TabNew, TabClose,
}

fn label(c: Cmd) -> &'static str {
    match c {
        Cmd::Sep => "",
        Cmd::Open => t!("Открыть", "Open"),
        Cmd::NewTab => t!("Открыть в новой вкладке", "Open in new tab"),
        Cmd::NewWindow => t!("Открыть в новом окне", "Open in new window"),
        Cmd::RunAs => t!("Запуск от имени администратора", "Run as administrator"),
        Cmd::Cut => t!("Вырезать", "Cut"),
        Cmd::Copy => t!("Копировать", "Copy"),
        Cmd::Paste => t!("Вставить", "Paste"),
        Cmd::CopyPath => t!("Копировать как путь", "Copy as path"),
        Cmd::Rename => t!("Переименовать", "Rename"),
        Cmd::Delete => t!("Удалить", "Delete"),
        Cmd::Pin => t!("Закрепить в быстром доступе", "Pin to Quick access"),
        Cmd::Unpin => t!("Открепить от быстрого доступа", "Unpin from Quick access"),
        Cmd::PinHere => t!("Закрепить текущую папку в быстром доступе", "Pin current folder to Quick access"),
        Cmd::UnpinHere => t!("Открепить текущую папку от быстрого доступа", "Unpin current folder from Quick access"),
        Cmd::Up => t!("Переместить выше", "Move up"),
        Cmd::Down => t!("Переместить ниже", "Move down"),
        Cmd::AddStart => t!("Добавить в «Пуск»", "Add to Start"),
        Cmd::RemoveStart => t!("Убрать из «Пуска»", "Remove from Start"),
        Cmd::Props => t!("Свойства", "Properties"),
        Cmd::More => t!("Другие параметры…", "More options…"),
        Cmd::NewFolder => t!("Создать папку", "New folder"),
        Cmd::NewFile => t!("Создать текстовый документ", "New text file"),
        Cmd::Refresh => t!("Обновить", "Refresh"),
        Cmd::Disk => t!("Использование диска (WinRust Disk)", "Disk usage (WinRust Disk)"),
        Cmd::Tabs => t!("Вкладки", "Tabs"),
        Cmd::TabNew => t!("Новая вкладка", "New tab"),
        Cmd::TabClose => t!("Закрыть вкладку", "Close tab"),
    }
}

// ---------- which entries a menu has: pure, see the tests ----------

/// What is selected, as plain facts.
#[derive(Clone, Copy, Default)]
pub struct Sel {
    pub n: usize,
    pub dirs: bool,    // all of it is folders
    pub apps: bool,    // all of it is .exe / .lnk files (what Start takes)
    pub runs: bool,    // one file that can be started elevated
    pub pinned: bool,  // all of it is in our Quick access
    pub started: bool, // all of it is in the topbar's Start
    pub clip: bool,    // there are files on the clipboard
    pub tabs: bool,    // the file manager has tabs (explorer.tabs)
    pub desk: bool,    // the desktop window, not a file manager one
}

/// Drops separators that would open or close the menu or follow one another (a group may come out empty).
fn tidy(v: Vec<Cmd>) -> Vec<Cmd> {
    let mut out: Vec<Cmd> = vec![];
    for c in v {
        if c != Cmd::Sep || out.last().is_some_and(|&l| l != Cmd::Sep) { out.push(c); }
    }
    if out.last() == Some(&Cmd::Sep) { out.pop(); }
    out
}

/// The menu of the selected files / folders.
pub fn item_cmds(s: &Sel) -> Vec<Cmd> {
    use Cmd::*;
    let mut v = vec![Open];
    if s.dirs && s.tabs && !s.desk { v.push(NewTab); }
    if s.dirs && !s.desk { v.push(NewWindow); } // on the desktop "Open" already is a new window
    if s.runs { v.push(RunAs); }
    v.extend([Sep, Cut, Copy]);
    if s.n == 1 && s.dirs && s.clip { v.push(Paste); } // into that folder
    v.extend([CopyPath, Sep]);
    if s.n == 1 { v.push(Rename); }
    v.extend([Delete, Sep]);
    if s.dirs { v.push(if s.pinned { Unpin } else { Pin }); }
    if s.apps { v.push(if s.started { RemoveStart } else { AddStart }); }
    v.extend([Sep, Props, Sep, More]);
    tidy(v)
}

/// The menu of the folder itself (right click on empty space). `pinned`: the folder is in our Quick access.
pub fn bg_cmds(desk: bool, pinned: bool, disk: bool) -> Vec<Cmd> {
    use Cmd::*;
    let mut v = vec![NewFolder, NewFile, Sep, Paste, Refresh, Sep];
    if !desk { v.extend([if pinned { UnpinHere } else { PinHere }, Tabs]); }
    if disk { v.push(Disk); }
    v.extend([Sep, Props, Sep, More]);
    tidy(v)
}

/// The menu of a folder or drive in the left pane.
pub fn nav_cmds(pinned: bool, tabs: bool) -> Vec<Cmd> {
    use Cmd::*;
    let mut v = vec![Open];
    if tabs { v.push(NewTab); }
    v.extend([NewWindow, Sep]);
    if pinned { v.extend([Unpin, Up, Down]); } else { v.push(Pin); }
    v.extend([Sep, Props]);
    v
}

/// What "Run as administrator" is offered for.
fn runnable(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["exe", "lnk", "bat", "cmd"].contains(&e.to_ascii_lowercase().as_str()))
}

/// "Copy as path" like Explorer's: every path in quotes, one per line.
fn as_path_text(paths: &[String]) -> String {
    paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\r\n")
}

/// Verbs of the stock shell menu that pin to Explorer's own Quick access / Start. They are taken out of "More options…":
/// ours are in our menu, and Explorer's lists are not the ones shown here.
pub fn os_pin_verb(verb: &str) -> bool {
    matches!(verb.to_ascii_lowercase().as_str(), "pintohome" | "unpinfromhome" | "pintostartscreen" | "unpinfromstartscreen" | "startpin" | "startunpin")
}

// ---------- showing a menu and doing what was picked ----------

/// Shows `cmds` as a popup (at `at`, else at the pointer) and returns the entry picked.
unsafe fn pick(h: HWND, cmds: &[Cmd], at: Option<POINT>, checked: &[Cmd], grey: &[Cmd]) -> Option<Cmd> {
    let m = CreatePopupMenu();
    for (i, &c) in cmds.iter().enumerate() {
        if c == Cmd::Sep { AppendMenuW(m, MF_SEPARATOR, 0, ptr::null()); continue; }
        let flags = MF_STRING | if checked.contains(&c) { MF_CHECKED } else { 0 } | if grey.contains(&c) { MF_GRAYED } else { 0 };
        AppendMenuW(m, flags, i + 1, w(label(c)).as_ptr());
    }
    let mut pt = at.unwrap_or(POINT { x: 0, y: 0 });
    if at.is_none() { GetCursorPos(&mut pt); }
    SetForegroundWindow(h); // or a click elsewhere would not close the menu
    let id = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, h, ptr::null());
    DestroyMenu(m);
    cmds.get((id as usize).wrapping_sub(1)).copied()
}

/// Where the Menu key opens the menu: under the first selected row (screen coordinates).
pub unsafe fn at_selection(list: HWND) -> Option<POINT> {
    let i = put(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
    if i < 0 { return None; }
    let mut r = RECT { left: LVIR_BOUNDS as i32, ..std::mem::zeroed() };
    put(list, LVM_GETITEMRECT, i as usize, &mut r as *mut _ as isize);
    let mut p = POINT { x: r.left + ui::sc(40), y: r.bottom };
    ClientToScreen(list, &mut p);
    Some(p)
}

fn text_of(p: &Path) -> String { p.to_string_lossy().into_owned() }

/// Right click / Menu key on the selection (nothing selected: the folder's menu). Never call with ST borrowed.
pub unsafe fn items(h: HWND, at: Option<POINT>) {
    let (list, dir, paths) = selected();
    if paths.is_empty() { return background(h); }
    let strs: Vec<String> = paths.iter().map(|p| text_of(p)).collect();
    let dirs = paths.iter().all(|p| p.is_dir());
    let apps = paths.iter().all(|p| quick::startable(p) && !p.is_dir());
    let s = Sel {
        n: paths.len(), dirs, apps, runs: paths.len() == 1 && !dirs && runnable(&paths[0]),
        pinned: dirs && { let q = quick::load(); strs.iter().all(|p| quick::find(&q, p).is_some()) },
        started: apps && quick::in_start(&strs),
        clip: paths.len() == 1 && dirs && sh::clip_get().is_some(),
        tabs: tabs_on(), desk: desk(),
    };
    let Some(cmd) = pick(h, &item_cmds(&s), at, &[], &[]) else { return };
    match cmd {
        Cmd::Open => {
            // files with their programs; the first folder here (the desktop: in a window), further folders next to it
            let (folders, files): (Vec<&PathBuf>, Vec<&PathBuf>) = paths.iter().partition(|p| p.is_dir());
            for f in files { open_item(h, f, false); }
            for (i, d) in folders.into_iter().enumerate() { if i == 0 { open_item(h, d, true) } else { open_new(h, d) } }
        }
        Cmd::NewTab => for p in &paths { tab_open(h, p.clone()); },
        Cmd::NewWindow => for p in &paths { new_window(p); },
        Cmd::RunAs => {
            let from = paths[0].parent().map(text_of).unwrap_or_default();
            ShellExecuteW(h, w("runas").as_ptr(), w(&strs[0]).as_ptr(), ptr::null(), w(&from).as_ptr(), SW_SHOWNORMAL);
        }
        Cmd::Cut | Cmd::Copy => { let _ = sh::clip_set(h, &paths, cmd == Cmd::Cut); }
        Cmd::Paste => { paste(h, &paths[0]); reload(h); } // a cut from this folder leaves it
        Cmd::CopyPath => { let _ = sh::clip_text(h, &as_path_text(&strs)); }
        Cmd::Rename => {
            let i = put(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
            if i >= 0 { SetFocus(list); put(list, LVM_EDITLABELW, i as usize, 0); }
        }
        Cmd::Delete => { let _ = sh::fileop(h, sh::Op::Delete, &paths, &dir); reload(h); }
        Cmd::Pin => quick::edit(|v| strs.iter().fold(false, |ch, p| quick::add(v, p) | ch)),
        Cmd::Unpin => quick::edit(|v| strs.iter().fold(false, |ch, p| quick::remove(v, p) | ch)),
        Cmd::AddStart | Cmd::RemoveStart => quick::start_set(&strs, cmd == Cmd::AddStart),
        Cmd::Props => if paths.len() == 1 { properties(&paths[0]) } else { let _ = sh::properties(h, &paths); },
        Cmd::More => { let _ = sh::show(h, &paths); reload(h); }
        _ => {}
    }
}

/// Right click on empty space: the folder's own menu.
pub unsafe fn background(h: HWND) {
    let dir = cur_dir();
    let here = text_of(&dir);
    let disk = tool("wiztree");
    let cmds = bg_cmds(desk(), !desk() && quick::pinned(&here), disk.is_some());
    let checked: &[Cmd] = if tabs_on() { &[Cmd::Tabs] } else { &[] };
    let grey: &[Cmd] = if sh::clip_get().is_some() { &[] } else { &[Cmd::Paste] };
    let Some(cmd) = pick(h, &cmds, None, checked, grey) else { return };
    match cmd {
        Cmd::NewFolder => new_entry(&dir, "New folder", true),
        Cmd::NewFile => new_entry(&dir, "New text document.txt", false),
        Cmd::Paste => paste(h, &dir),
        Cmd::Refresh => {}
        Cmd::PinHere => return quick::edit(|v| quick::add(v, &here)),
        Cmd::UnpinHere => return quick::edit(|v| quick::remove(v, &here)),
        Cmd::Tabs => return set_tabs(h, !tabs_on()),
        Cmd::Disk => { // wiztree takes the drive letter and asks for admin rights itself
            let drive: String = here.chars().take(2).collect();
            if let Some(exe) = disk { let _ = std::process::Command::new(exe).arg(drive).spawn(); }
            return;
        }
        Cmd::Props => return properties(&dir),
        Cmd::More => { let _ = sh::show_bg(h, &dir); }
        _ => return,
    }
    reload(h);
}

/// Right click in the left pane: on a folder or drive (`idx`), or on a heading / empty space (-1: about the folder shown).
pub unsafe fn nav(h: HWND, idx: isize) {
    let list = quick::load();
    let Some(path) = usize::try_from(idx).ok().and_then(ui::nav_path) else {
        let here = text_of(&cur_dir());
        match pick(h, &[if quick::find(&list, &here).is_some() { Cmd::UnpinHere } else { Cmd::PinHere }], None, &[], &[]) {
            Some(Cmd::PinHere) => quick::edit(|v| quick::add(v, &here)),
            Some(Cmd::UnpinHere) => quick::edit(|v| quick::remove(v, &here)),
            _ => {}
        }
        return;
    };
    let p = text_of(&path);
    let at = quick::find(&list, &p);
    let mut grey = vec![];
    if at == Some(0) { grey.push(Cmd::Up); }
    if at.is_some_and(|i| i + 1 == list.len()) { grey.push(Cmd::Down); }
    let Some(cmd) = pick(h, &nav_cmds(at.is_some(), tabs_on()), None, &[], &grey) else { return };
    match cmd {
        Cmd::Open => load(h, path),
        Cmd::NewTab => tab_open(h, path),
        Cmd::NewWindow => new_window(&path),
        Cmd::Pin => quick::edit(|v| quick::add(v, &p)),
        Cmd::Unpin => quick::edit(|v| quick::remove(v, &p)),
        Cmd::Up | Cmd::Down => quick::edit(|v| quick::shift(v, &p, cmd == Cmd::Up)),
        Cmd::Props => properties(&path),
        _ => {}
    }
}

/// Right click on the top part of the window: tabs on / off for every file manager window; `tab` = the tab under the
/// pointer, -1 for none.
pub unsafe fn chrome(h: HWND, tab: isize) {
    let (on, n) = (tabs_on(), ST.with_borrow(|st| st.tabs.len()));
    let mut cmds = vec![];
    if on {
        cmds.push(Cmd::TabNew);
        if tab >= 0 && n > 1 { cmds.push(Cmd::TabClose); }
        cmds.push(Cmd::Sep);
    }
    cmds.push(Cmd::Tabs);
    match pick(h, &cmds, None, if on { &[Cmd::Tabs] } else { &[] }, &[]) {
        Some(Cmd::TabNew) => tab_new(),
        Some(Cmd::TabClose) => tab_close_at(h, tab as usize),
        Some(Cmd::Tabs) => set_tabs(h, !on),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{Cmd::*, *};

    #[test]
    fn a_folder_can_be_opened_elsewhere_and_pinned() {
        let s = Sel { n: 1, dirs: true, tabs: true, ..Sel::default() };
        assert_eq!(item_cmds(&s), [Open, NewTab, NewWindow, Sep, Cut, Copy, CopyPath, Sep, Rename, Delete, Sep, Pin, Sep, Props, Sep, More]);
        // tabs switched off: no "Open in new tab"; pinned already: unpin; files on the clipboard: paste into it
        let s = Sel { tabs: false, pinned: true, clip: true, ..s };
        assert_eq!(item_cmds(&s), [Open, NewWindow, Sep, Cut, Copy, Paste, CopyPath, Sep, Rename, Delete, Sep, Unpin, Sep, Props, Sep, More]);
        // on the desktop a folder opens in a window anyway
        let s = Sel { n: 1, dirs: true, tabs: true, desk: true, ..Sel::default() };
        assert_eq!(item_cmds(&s)[..2], [Open, Sep]);
    }

    #[test]
    fn a_program_can_be_elevated_and_put_into_start() {
        let s = Sel { n: 1, apps: true, runs: true, ..Sel::default() };
        assert_eq!(item_cmds(&s), [Open, RunAs, Sep, Cut, Copy, CopyPath, Sep, Rename, Delete, Sep, AddStart, Sep, Props, Sep, More]);
        assert!(item_cmds(&Sel { started: true, ..s }).contains(&RemoveStart));
    }

    #[test]
    fn plain_files_and_several_at_once() {
        // a text file: nothing to pin; several items: no rename
        assert_eq!(item_cmds(&Sel { n: 1, ..Sel::default() }), [Open, Sep, Cut, Copy, CopyPath, Sep, Rename, Delete, Sep, Props, Sep, More]);
        assert_eq!(item_cmds(&Sel { n: 3, ..Sel::default() }), [Open, Sep, Cut, Copy, CopyPath, Sep, Delete, Sep, Props, Sep, More]);
        let all = item_cmds(&Sel { n: 2, dirs: true, clip: true, tabs: true, ..Sel::default() });
        assert!(all.contains(&NewTab) && all.contains(&Pin) && !all.contains(&Paste) && !all.contains(&Rename));
        assert_eq!(all.last(), Some(&More)); // the stock menu is always the last entry
    }

    #[test]
    fn the_folder_menu() {
        assert_eq!(bg_cmds(false, false, true), [NewFolder, NewFile, Sep, Paste, Refresh, Sep, PinHere, Tabs, Disk, Sep, Props, Sep, More]);
        assert_eq!(bg_cmds(false, true, false)[6], UnpinHere);
        assert_eq!(bg_cmds(true, false, false), [NewFolder, NewFile, Sep, Paste, Refresh, Sep, Props, Sep, More]); // the desktop: no tabs, no pane
    }

    #[test]
    fn the_pane_menu() {
        assert_eq!(nav_cmds(true, true), [Open, NewTab, NewWindow, Sep, Unpin, Up, Down, Sep, Props]);
        assert_eq!(nav_cmds(false, false), [Open, NewWindow, Sep, Pin, Sep, Props]); // a drive that is not pinned, tabs off
    }

    #[test]
    fn separators_never_double_up_or_end_the_menu() {
        assert_eq!(tidy(vec![Sep, Open, Sep, Sep, Props, Sep]), [Open, Sep, Props]);
        assert!(tidy(vec![Sep, Sep]).is_empty());
    }

    #[test]
    fn helpers() {
        assert_eq!(as_path_text(&["C:\\a b\\c.txt".into(), "D:\\x".into()]), "\"C:\\a b\\c.txt\"\r\n\"D:\\x\"");
        assert!(runnable(Path::new("C:\\a\\Setup.EXE")) && runnable(Path::new("x.lnk")) && !runnable(Path::new("x.txt")));
        assert!(os_pin_verb("PinToHome") && os_pin_verb("unpinfromstartscreen") && !os_pin_verb("properties") && !os_pin_verb("taskbarpin"));
        assert_eq!(label(More), "Другие параметры…"); // tests run in Russian
    }
}
