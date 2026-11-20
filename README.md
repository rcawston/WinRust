<p align="center"><img src="assets/logo.svg" alt="WinRust" width="160"></p>

# WinRust

**A Windows shell written in Rust: faster and lighter than the stock one.** Small native programs (no .NET, no Electron) that replace parts of Windows: the taskbar, File Explorer, file search, Task Manager, plus a disk cleaner, a disk map and a photo viewer. One installer puts them in, one button takes them all out.

English | [Русский](README.ru.md)

<p align="center"><img src="assets/th.jpg" alt="WinRust" width="480"></p>

> **Read this first.** WinRust turns off Windows Search, hides the taskbar and can turn off Explorer as the shell. The exe is not code-signed, so SmartScreen will warn you. A restore point is created before install, but try it on a spare machine or a VM first.

## Install

1. Download `WinRust.exe` from the [latest release](https://github.com/Lemxy/WinRust/releases/latest) (Windows 10/11, x64).
2. Run it as administrator.
3. Tick the components you want (all are ticked by default) and press **Install WinRust**. They are installed in the right order; one that fails or is skipped does not stop the others, the window lists what went wrong.

Windows Search is optional: on a stripped Windows (for example a debloated LTSC image without the Windows Search service) that step is skipped with a warning instead of failing the install.

If UAC asks for **another administrator's** password, the per-user settings (startup entries, scheduled tasks, PowerShell profile, search permissions) still go to the user who is signed in, not to the administrator you typed. If the installer cannot tell who that is, it stops before changing anything.

**Update:** download the new `WinRust.exe` from the releases page, run it as administrator and press **Update WinRust**. Installed plugins are updated, missing ones installed; your settings and the saved original Windows values stay.

The installer is in English. The **Русский** button in its corner (or "WinRust apps in Русский" in the bar's right-click menu) switches every WinRust program to Russian.

## Remove

- **Remove all** in the installer window undoes everything: Windows Search, the taskbar, Explorer and the services come back.
- One plugin at a time: `plugin-launcher uninstall <plugin>` (see [Command line](#command-line)).
- **If the screen is left without a taskbar or desktop:** press Ctrl+Alt+Del → Task Manager → Run new task → `explorer.exe`, then run the installer and press Remove all.

## What gets installed

| Plugin | What you get | What it changes in Windows |
|---|---|---|
| **rust-search** | File search over all NTFS drives, Alt+Space | Disables the Windows Search service, blocks SearchHost.exe, hides the taskbar search box |
| **rtaskmgr-win32** | Task Manager (Ctrl+Shift+Esc opens it) | `taskmgr.exe` is redirected to it (the system file is untouched). Also installs the **rs-du** disk cleaner |
| **topbar** | A Windows 11-style bottom bar: windows, clock, volume, menus | Hides the stock taskbar. Also installs **reshell** (hotkeys: Win+1/2/3 and Win+\` switch desktops, Win+D shows the desktop, Win+I opens Settings, Win+E the file manager; brings the taskbar back if the bar dies), **rs-du**, and a block in your PowerShell profile (`zoxide`, `grep`=rg, `ll`=eza, `cat`=bat; does nothing if they are missing) |
| **rust-explorer** | A file manager with tabs, and the desktop | Folders and drives open in it; **Explorer stops being the shell** (comes back by itself if topbar fails). Disables PcaSvc, InventorySvc, lfsvc, cbdhsvc, sets DoSvc to manual, merges svchost processes. Requires topbar |
| **wiztree** | A disk-usage map ("WinRust Disk" in Start) | Nothing, apart from the file and a shortcut |
| **photoview** | A fast photo viewer | Adds it to "Open with" (it does not become the default) |
| **debloat** | Off: telemetry, Copilot, Recall, Start ads, Edge in the background | Services and registry policies; old values are saved and restored on uninstall. **Command line only:** `install debloat --yes` |
| **purge** | Removes the bundled junk like Tiny11 does: Clipchamp, News, Weather, Xbox overlays, Teams, Cortana, Widgets, Skype and more, plus telemetry tasks | **IRREVERSIBLE, no undo.** Apps are removed for all users and from the image; the Store, Settings, Start and runtimes stay. Run `purge.exe` first: it only lists. **Command line only:** `install purge --yes` (OneDrive and Edge: `purge.exe --apply --onedrive --edge`) |
| chrome-rs | A browser | Not installable yet |

Every change is reversible: uninstall puts back the values that were there before.

## Styles

**Style** is in the bar's right-click menu. The choice applies at once to the bar, the file manager (rust-explorer) and the search window (Alt+Space):

| Style | Look |
|---|---|
| **Normal** | Windows 11 blue, the bar at the bottom |
| **Terminal** | Green on black, Consolas, square corners |
| **Floating dock** | A pill-shaped bar in the middle; maximized windows get the whole screen, the dock slides away over them and comes back at the bottom edge |
| **Tiling** | A thin bar at the top, amber accent, square corners; "Tile the windows" puts the open windows side by side |

The choice is stored in `%ProgramData%\WinRust\style.txt`.

## Settings

Right-click an empty spot of the bar (or the Start button). **Bar settings**, **Start menu** and **Style** are submenus there; a switch applies at once and the menu stays open for the next one. Everything is kept in `%APPDATA%\WinRust\config.txt` as `key = value` lines, which you can also edit by hand ("Open the settings file" writes every key with an explanation). Saved edits apply within a second.

| Key | Default | What it does |
|---|---|---|
| `topbar.autohide` | `0` | `1`: the bar slides away when the pointer leaves it and comes back at the screen edge. It then reserves no screen space, so nothing static stays on an OLED screen |
| `topbar.autohide_delay_ms` | `800` | How long the bar stays after the pointer left it |
| `topbar.align` | `center` | `left`: Start in the corner, then search and the app buttons |
| `topbar.labels` | `0` | `1`: an open window gets a wide button with its title; apps that are only pinned stay icons |
| `topbar.label_width` | `160` | Width of such a button (pixels at 100% scaling); they shrink when the bar is full |
| `topbar.search` | `1` | `0`: no search button on the bar |
| `topbar.gear` | `0` | `1`: a gear button at the left end (left click: Style, right click: Bar settings) |
| `desktop.autohide_icons` | `0` | `1`: desktop icons are hidden until you click the desktop |
| `desktop.icons_delay_s` | `5` | Seconds without mouse activity on the desktop after which the icons hide again |
| `start.buttons` | `lock,sleep,restart,shutdown` | Buttons at the bottom of Start: any of `lock`, `signout`, `sleep`, `hibernate`, `restart`, `shutdown` |
| `start.power_menu` | `0` | `1`: one power button that opens those actions as a menu |
| `explorer.tabs` | `1` | `0`: the file manager has no tabs, one folder per window |

Also on the bar:

- **Start menu → Import what Windows has pinned** adds the Start tiles of Windows and the Quick Launch shortcuts to Start. Start's power buttons show a tooltip.
- **Shift+click** an app's button starts the app once more (a new window, when the app supports it).
- The language button shows only when more than one keyboard layout is installed.
- **Network:** on a PC without Wi-Fi the icon, the quick panel's tile and the network panel are about the cable: connected or not, link speed, address, "no internet". With Wi-Fi, a wired connection is listed above the Wi-Fi networks.
- **Bluetooth:** the quick panel's tile switches the radio and names the connected device; its arrow opens the Bluetooth settings.
- **Sound:** right-click the volume icon (or the arrow next to the volume slider) for the sound panel: pick the output and the input device (it becomes the Windows default), microphone level and mute, links to the sound settings and the volume mixer. Middle click on the volume icon mutes.

In the file manager and on the desktop:

- **Right-click menus are WinRust's own:** open (in a new tab or window), run as administrator, cut / copy / paste, copy as path, rename, delete, properties. "Pin to Quick access" and "Add to Start" act on WinRust's lists, not on Explorer's. The **Context menu** submenu holds what Windows itself would add for that file or folder: the entries from the registry and from installed extensions (Open with, Send to, Edit, Print, 7-Zip, Git…).
- **Quick access** in the left pane starts as a copy of the folders pinned in Windows and is yours from then on: right-click a folder to pin or unpin it, right-click it in the pane to move it up or down. The list is `%APPDATA%\rust-explorer\quick-access.txt`, one folder per line.
- **Tabs** can be switched off: right-click the top of the window or the folder background → Tabs (the `explorer.tabs` key). Ctrl+T then opens a new window.

## Benchmark

One-off measurements on one machine (Windows 11 IoT Enterprise 10.0.26300, RTX 4060 Laptop, NTFS). Treat them as orders of magnitude, not lab numbers.

| What | Windows | WinRust | Difference |
|---|---|---|---|
| File search (rust-search over all of C: vs `dir /s` over one folder) | 8,340 ms | 45 ms | **99.5%** faster |
| Same vs `Get-ChildItem -Recurse` | 16,200 – 28,600 ms | 45 ms | **99.7–99.8%** faster |
| Open a folder window (rust-explorer, median of 5 runs) | 420 ms | 60 ms | **86%** faster |
| Shell memory (topbar + rust-explorer + rs-shell + reshell vs explorer.exe) | 366 MB | 46 MB | **87%** less |
| Same, plus the search index (53 MB) | 366 MB | 99 MB | **73%** less |
| File manager binary size | 3,280 KB | 498 KB | **85%** smaller |
| Task manager binary size | 5,791 KB | 869 KB | **85%** smaller |
| Idle CPU of the bar (topbar after optimization, % of one core) | 0.00–0.10% (explorer) | 0.05–0.10% | on par with Explorer |

In short: **85–99% faster or smaller in every size/speed test, and equal on idle CPU.** Caveats: rs searches a ready-made MFT index (it pays 53 MB of memory for that), while `dir` and PowerShell walk the disk from scratch; explorer.exe was still running with its windows during the test, so the memory gain is slightly overstated; Windows Search and the stock Task Manager were not compared.

## Screenshots

**Taskbar**

![Taskbar on the desktop](docs/screenshots/taskbar.webp)
![Taskbar menu](docs/screenshots/taskbar-menu.png)
![Bar menu, new look](docs/screenshots/topbar-menu-new.png)

**Task Manager** (Russian UI)

![Task Manager](docs/screenshots/task-manager.webp)

**File manager**

![File manager](docs/screenshots/explorer.png)

---

## For developers

### Layout
```
src/            the installer (plugin-launcher.exe)
  main.rs         the plugin table PLUGINS: name, texts, install/uninstall pair, icon
  install/        what each plugin changes and how it is undone (one file per plugin)
  cli.rs, ui.rs   the command line and the window; both call the same install functions
common/         winrust-common: language switch (t!/tf!), UI styles and palettes, reg.exe output parser, NTFS MFT reader
plugins/        sources of every plugin, each its own crate
ReShell/        reshell (desktop hotkeys, topbar safety net), ships with topbar
payload.txt     the exes embedded into the installer, and where the build puts them
payload/        those exes once built (not in git)
scripts/        build, update, release
```
Everything is one cargo workspace (one `Cargo.lock`, every exe in `target\release`). `plugins/rust-search` and `plugins/chrome-rs` build separately because they need `panic = "unwind"` (see `Cargo.toml`).

### Build
Needs Windows x64, Rust stable (1.85+) with the MSVC toolchain.
```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-all.ps1
```
Result: `target\release\plugin-launcher.exe`, with every plugin inside. Close a running installer first. Tests: `cargo test --workspace` (and `cargo test` in `plugins\rust-search`). CI runs the same on every push (`.github/workflows/build.yml`).

| Script | What it does |
|---|---|
| `build-all.ps1` | everything: plugins → `payload\` → installer |
| `build-plugins.ps1` | plugins only → `payload\` (no installer rebuild) |
| `update-plugin.ps1 <plugin>` | rebuild and reinstall one installed plugin without rebuilding the installer (asks for admin via UAC) |
| `user-mode.ps1`, `dev-update.ps1` | developer mode: run topbar, rust-explorer, rs-shell and reshell from `%LOCALAPPDATA%` on top of Explorer, swap them without admin |
| `bench.ps1` | the search and Task Manager benchmark |
| `publish.ps1`, `release.ps1` | commit and push; GitHub release with `WinRust.exe` |

### Command line
From a terminal **with administrator rights**:
```
plugin-launcher list
plugin-launcher install <plugin>... --yes (one or more names, or `all` for what the window installs; without --yes it only prints the warnings and changes nothing)
plugin-launcher update <plugin>          (reinstall from payload\ next to the exe)
plugin-launcher uninstall <plugin>
plugin-launcher rollback --yes           (remove everything)
```
Several plugins in one command are installed in the table's order (topbar before rust-explorer); one that fails does not stop the rest. Exit codes: 0 success, 1 error (at least one plugin failed), 2 bad arguments or no `--yes`. With no arguments the window opens. Install log: `%ProgramData%\plugin-launcher\install.log`.

### Adding a plugin
1. A crate in `plugins/<name>`, added to `members` in the root `Cargo.toml`.
2. A line in `payload.txt`.
3. `src/install/<name>.rs` with an install/uninstall pair (and a `mod` line in `src/install/mod.rs`). Save the original values before changing anything, so uninstall can put them back.
4. An entry in `PLUGINS` (`src/main.rs`). The window, its height and the command line pick it up on their own.
