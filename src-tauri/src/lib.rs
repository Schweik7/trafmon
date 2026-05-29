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

/// Position the detail popup just below the main widget and show it.
#[tauri::command]
fn show_detail(app: AppHandle) {
    let Some(detail) = app.get_webview_window("detail") else { return };
    if let Some(main) = app.get_webview_window("main") {
        if let (Ok(pos), Ok(size)) = (main.outer_position(), main.outer_size()) {
            let _ = detail.set_position(PhysicalPosition::new(pos.x, pos.y + size.height as i32 + 2));
        }
    }
    let _ = detail.set_ignore_cursor_events(true);
    let _ = detail.show();
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
fn relaunch_as_admin(app: &AppHandle) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &format!("Start-Process -FilePath '{}' -Verb RunAs", exe.display()),
            ])
            .spawn();
        app.exit(0);
    }
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
            let menu = menu_builder.separator().item(&quit).build()?;

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
