mod monitor;
mod netproc;

use monitor::{MonitorState, NetProcInfo, NetStats, SharedState};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State};

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
fn show_about() {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};

    std::thread::spawn(|| {
        let title = HSTRING::from("关于 trafmon");
        let body = HSTRING::from(format!(
            "trafmon  v{}\n\n作者：Schweik7\n仓库：https://github.com/Schweik7/trafmon",
            env!("CARGO_PKG_VERSION")
        ));
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(MonitorState::new()) as SharedState)
        .invoke_handler(tauri::generate_handler![
            get_net_stats,
            get_net_processes,
            set_interface,
            show_detail,
            hide_detail,
            place_detail,
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
            // Only meaningful when not elevated: per-process speed needs admin.
            let elevate =
                MenuItemBuilder::with_id("elevate", "显示详情（需以管理员身份运行）").build(app)?;

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

            let mut menu_builder = MenuBuilder::new(app)
                .item(&toggle)
                .item(&nic_menu)
                .item(&opacity_menu)
                .item(&theme);
            if !elevated {
                menu_builder = menu_builder.separator().item(&elevate);
            }
            let menu = menu_builder.separator().item(&about).item(&quit).build()?;

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
                        show_about();
                    } else if id == "elevate" {
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
