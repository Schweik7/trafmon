mod monitor;
mod netproc;

use monitor::{MonitorState, NetProcInfo, NetStats, SharedState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::menu::{
    CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder, Submenu,
    SubmenuBuilder,
};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, Wry};
use tauri_plugin_autostart::ManagerExt;

#[tauri::command]
fn get_net_stats(state: State<'_, SharedState>) -> NetStats {
    state.lock().unwrap().refresh_net()
}

#[tauri::command]
fn get_net_processes(state: State<'_, SharedState>) -> NetProcInfo {
    state.lock().unwrap().refresh_net_processes()
}

#[tauri::command]
fn set_interface(name: String, state: State<'_, SharedState>) {
    state.lock().unwrap().set_interface(name);
}

/// Position the detail popup adjacent to the main widget, picking the side with
/// room so a corner-docked widget still shows its panel fully on-screen.
/// `panel_w`/`panel_h` are physical pixels.
fn place_detail_window(app: &AppHandle, panel_w: i32, panel_h: i32) {
    let (Some(detail), Some(main)) =
        (app.get_webview_window("detail"), app.get_webview_window("main"))
    else {
        return;
    };
    let (Ok(mpos), Ok(msize)) = (main.outer_position(), main.outer_size()) else {
        return;
    };
    let scale = main.scale_factor().unwrap_or(1.0);
    let gap = (4.0 * scale).round() as i32;

    // Work area excludes the taskbar; fall back to a generous rect.
    let (wx, wy, ww, wh) = match main.current_monitor() {
        Ok(Some(m)) => {
            let wa = m.work_area();
            (wa.position.x, wa.position.y, wa.size.width as i32, wa.size.height as i32)
        }
        _ => (mpos.x, mpos.y, 1 << 20, 1 << 20),
    };
    let (work_right, work_bottom) = (wx + ww, wy + wh);
    let main_h = msize.height as i32;

    // Vertical: fit below if it fits, else above, else whichever side is roomier.
    let space_below = work_bottom - (mpos.y + main_h) - gap;
    let space_above = mpos.y - wy - gap;
    let open_below = if panel_h <= space_below {
        true
    } else if panel_h <= space_above {
        false
    } else {
        space_below >= space_above
    };
    let y = if open_below {
        mpos.y + main_h + gap
    } else {
        mpos.y - panel_h - gap
    }
    .clamp(wy, (work_bottom - panel_h).max(wy));

    // Horizontal: align the panel's left edge with the widget, then pull it back
    // on-screen if it spills past either work-area edge.
    let mut x = mpos.x;
    if x + panel_w > work_right {
        x = work_right - panel_w;
    }
    x = x.clamp(wx, (work_right - panel_w).max(wx));

    let _ = detail.set_position(PhysicalPosition::new(x, y));
}

/// Show the detail popup, positioning it relative to the widget. The frontend
/// calls `place_detail` again after it resizes to content.
#[tauri::command]
fn show_detail(app: AppHandle) {
    let Some(detail) = app.get_webview_window("detail") else { return };
    if let Ok(size) = detail.outer_size() {
        place_detail_window(&app, size.width as i32, size.height as i32);
    }
    let _ = detail.set_ignore_cursor_events(true);
    let _ = detail.show();
}

/// Reposition the detail popup for its current content size (logical pixels).
#[tauri::command]
fn place_detail(app: AppHandle, width: f64, height: f64) {
    let scale = app
        .get_webview_window("main")
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0);
    place_detail_window(&app, (width * scale).round() as i32, (height * scale).round() as i32);
}

#[tauri::command]
fn hide_detail(app: AppHandle) {
    if let Some(detail) = app.get_webview_window("detail") {
        let _ = detail.hide();
    }
}

/// Whether this process has an elevated (Administrator) token.
fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Relaunch the current executable elevated (UAC), then exit this instance.
/// Uses ShellExecuteW("runas") so the elevated process is created by the system
/// UAC service — it survives this process exiting (unlike a spawned child, which
/// WebView2's job object would kill when we exit).
fn relaunch_as_admin(app: &AppHandle) {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    if let Ok(exe) = std::env::current_exe() {
        let file = HSTRING::from(exe.as_os_str());
        unsafe {
            ShellExecuteW(
                None,
                w!("runas"),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
        app.exit(0);
    }
}

/// Show a native "About" dialog with author, version and repository. Runs on
/// its own thread so the modal box doesn't block the tray event loop.
fn show_about(en: bool) {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};

    let (title, body) = if en {
        (
            "About trafmon".to_string(),
            format!(
                "trafmon  v{}\n\nAuthor: Schweik7\nRepository: https://github.com/Schweik7/trafmon",
                env!("CARGO_PKG_VERSION")
            ),
        )
    } else {
        (
            "关于 trafmon".to_string(),
            format!(
                "trafmon  v{}\n\n作者：Schweik7\n仓库：https://github.com/Schweik7/trafmon",
                env!("CARGO_PKG_VERSION")
            ),
        )
    };

    std::thread::spawn(move || {
        let title = HSTRING::from(title);
        let body = HSTRING::from(body);
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(body.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    });
}

/// Tray menu handles plus the current language, so the labels can be re-titled
/// live when the user switches language (and so the About box matches).
struct TrayUi {
    toggle: MenuItem<Wry>,
    theme: MenuItem<Wry>,
    about: MenuItem<Wry>,
    quit: MenuItem<Wry>,
    /// Always present. Label depends on `is_elevated`: when not elevated, prompts
    /// the user to relaunch as admin; when elevated, offers a plain restart
    /// (useful when the long-running ETW session degrades).
    relaunch: MenuItem<Wry>,
    is_elevated: bool,
    nic: Submenu<Wry>,
    opacity: Submenu<Wry>,
    language: Submenu<Wry>,
    lang_zh: CheckMenuItem<Wry>,
    lang_en: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    is_en: AtomicBool,
}

/// Re-title every translatable tray label for the chosen language and tick the
/// matching language entry.
fn apply_language(ui: &TrayUi, en: bool) {
    let _ = ui
        .toggle
        .set_text(if en { "Show / Hide" } else { "显示 / 隐藏" });
    let _ = ui
        .theme
        .set_text(if en { "Toggle theme" } else { "切换主题" });
    let _ = ui.about.set_text(if en { "About" } else { "关于" });
    let _ = ui.quit.set_text(if en { "Quit" } else { "退出" });
    let _ = ui.relaunch.set_text(match (ui.is_elevated, en) {
        (false, false) => "显示详情（需以管理员身份运行）",
        (false, true) => "Show details (run as Administrator)",
        (true, false) => "重启程序",
        (true, true) => "Restart",
    });
    let _ = ui.nic.set_text(if en { "Network card" } else { "网卡" });
    let _ = ui.opacity.set_text(if en { "Opacity" } else { "不透明度" });
    let _ = ui.language.set_text(if en { "Language" } else { "语言" });
    let _ = ui.autostart.set_text(if en {
        "Launch at startup"
    } else {
        "开机启动"
    });
    let _ = ui.lang_zh.set_checked(!en);
    let _ = ui.lang_en.set_checked(en);
    ui.is_en.store(en, Ordering::Relaxed);
}

/// Set the UI language. Called by the frontend on startup (to sync the tray with
/// the persisted preference) and whenever the language entry is clicked.
#[tauri::command]
fn set_language(app: AppHandle, lang: String) {
    if let Some(ui) = app.try_state::<TrayUi>() {
        apply_language(ui.inner(), lang == "en");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(Mutex::new(MonitorState::new()) as SharedState)
        .invoke_handler(tauri::generate_handler![
            get_net_stats,
            get_net_processes,
            set_interface,
            show_detail,
            hide_detail,
            place_detail,
            set_language,
        ])
        .setup(|app| {
            // Snapshot interfaces for the tray submenu.
            let (ifaces, current) = {
                let st = app.state::<SharedState>();
                let g = st.lock().unwrap();
                (g.interface_list(), g.current_interface())
            };

            let elevated = is_elevated();

            let toggle = MenuItemBuilder::with_id("toggle", "显示 / 隐藏").build(app)?;
            let theme = MenuItemBuilder::with_id("theme", "切换主题").build(app)?;
            let about = MenuItemBuilder::with_id("about", "关于").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            // Always present. Non-elevated: relaunches via UAC so per-process
            // speed works. Elevated: plain restart, useful when the long-running
            // ETW session stops emitting events (see `relaunch_as_admin`:
            // ShellExecuteW("runas") from an already-elevated process re-launches
            // elevated without an extra UAC prompt).
            let relaunch_label = if elevated {
                "重启程序"
            } else {
                "显示详情（需以管理员身份运行）"
            };
            let relaunch = MenuItemBuilder::with_id("relaunch", relaunch_label).build(app)?;

            // Network-card submenu (checkable, current one ticked).
            let mut nic_items = Vec::new();
            for name in &ifaces {
                let it = CheckMenuItemBuilder::with_id(format!("nic:{name}"), name)
                    .checked(name == &current)
                    .build(app)?;
                nic_items.push(it);
            }
            let mut nic_builder = SubmenuBuilder::new(app, "网卡");
            for it in &nic_items {
                nic_builder = nic_builder.item(it);
            }
            let nic_menu = nic_builder.build()?;

            // Opacity submenu.
            let levels = [
                ("opacity:100", "100%", true),
                ("opacity:85", "85%", false),
                ("opacity:70", "70%", false),
                ("opacity:50", "50%", false),
            ];
            let mut op_items = Vec::new();
            for (id, label, checked) in levels {
                op_items.push(
                    CheckMenuItemBuilder::with_id(id, label)
                        .checked(checked)
                        .build(app)?,
                );
            }
            let mut op_builder = SubmenuBuilder::new(app, "不透明度");
            for it in &op_items {
                op_builder = op_builder.item(it);
            }
            let opacity_menu = op_builder.build()?;

            // Language submenu (default 中文; the frontend re-syncs on startup
            // from its persisted preference).
            let lang_zh = CheckMenuItemBuilder::with_id("lang:zh", "中文")
                .checked(true)
                .build(app)?;
            let lang_en = CheckMenuItemBuilder::with_id("lang:en", "English")
                .checked(false)
                .build(app)?;
            let lang_menu = SubmenuBuilder::new(app, "语言")
                .item(&lang_zh)
                .item(&lang_en)
                .build()?;

            // Auto-start at boot. Reflects the current HKCU Run entry.
            let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
            let autostart = CheckMenuItemBuilder::with_id("autostart", "开机启动")
                .checked(autostart_enabled)
                .build(app)?;

            let menu = MenuBuilder::new(app)
                .item(&toggle)
                .item(&nic_menu)
                .item(&opacity_menu)
                .item(&theme)
                .item(&lang_menu)
                .item(&autostart)
                .separator()
                .item(&relaunch)
                .separator()
                .item(&about)
                .item(&quit)
                .build()?;

            app.manage(TrayUi {
                toggle: toggle.clone(),
                theme: theme.clone(),
                about: about.clone(),
                quit: quit.clone(),
                relaunch: relaunch.clone(),
                is_elevated: elevated,
                nic: nic_menu.clone(),
                opacity: opacity_menu.clone(),
                language: lang_menu.clone(),
                lang_zh: lang_zh.clone(),
                lang_en: lang_en.clone(),
                autostart: autostart.clone(),
                is_en: AtomicBool::new(false),
            });

            let nic_cb = nic_items.clone();
            let op_cb = op_items.clone();

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("trafmon")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| {
                    let id = event.id().as_ref();
                    if id == "quit" {
                        app.exit(0);
                    } else if id == "about" {
                        let en = app
                            .try_state::<TrayUi>()
                            .map(|ui| ui.is_en.load(Ordering::Relaxed))
                            .unwrap_or(false);
                        show_about(en);
                    } else if id == "lang:zh" || id == "lang:en" {
                        let en = id == "lang:en";
                        if let Some(ui) = app.try_state::<TrayUi>() {
                            apply_language(ui.inner(), en);
                        }
                        let _ = app.emit("set-lang", if en { "en" } else { "zh" });
                    } else if id == "autostart" {
                        let mgr = app.autolaunch();
                        let target = !mgr.is_enabled().unwrap_or(false);
                        let _ = if target {
                            mgr.enable()
                        } else {
                            mgr.disable()
                        };
                        if let Some(ui) = app.try_state::<TrayUi>() {
                            let _ = ui
                                .autostart
                                .set_checked(mgr.is_enabled().unwrap_or(target));
                        }
                    } else if id == "relaunch" {
                        relaunch_as_admin(app);
                    } else if id == "toggle" {
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(true) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    } else if id == "theme" {
                        let _ = app.emit("toggle-theme", ());
                    } else if let Some(name) = id.strip_prefix("nic:") {
                        app.state::<SharedState>()
                            .lock()
                            .unwrap()
                            .set_interface(name.to_string());
                        for it in &nic_cb {
                            let _ = it.set_checked(it.id().as_ref() == id);
                        }
                    } else if let Some(pct) = id.strip_prefix("opacity:") {
                        if let Ok(p) = pct.parse::<f64>() {
                            let _ = app.emit("set-opacity", p / 100.0);
                            for it in &op_cb {
                                let _ = it.set_checked(it.id().as_ref() == id);
                            }
                        }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
