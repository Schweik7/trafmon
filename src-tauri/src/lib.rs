mod monitor;
mod netproc;

use monitor::{MonitorState, NetProcInfo, NetStats, SharedState};
use std::sync::Mutex;
use tauri::State;

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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
