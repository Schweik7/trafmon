mod monitor;
mod netproc;

use monitor::{MonitorState, NetProcInfo, NetStats, SharedState};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State};

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(MonitorState::new()) as SharedState)
        .invoke_handler(tauri::generate_handler![
            get_net_stats,
            get_net_processes,
            set_interface,
        ])
        .setup(|app| {
            // Snapshot interfaces for the tray submenu.
            let (ifaces, current) = {
                let st = app.state::<SharedState>();
                let g = st.lock().unwrap();
                (g.interface_list(), g.current_interface())
            };

            let toggle = MenuItemBuilder::with_id("toggle", "显示 / 隐藏").build(app)?;
            let theme = MenuItemBuilder::with_id("theme", "切换主题").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;

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

            let menu = MenuBuilder::new(app)
                .item(&toggle)
                .item(&nic_menu)
                .item(&opacity_menu)
                .item(&theme)
                .separator()
                .item(&quit)
                .build()?;

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
