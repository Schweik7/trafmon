use once_cell::sync::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use sysinfo::{Networks, System};

#[derive(Serialize, Clone)]
pub struct NetStats {
    pub interface: String,
    pub upload_bps: u64,
    pub download_bps: u64,
    pub interfaces: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct ProcessEntry {
    pub name: String,
    pub value: f32,
}

#[derive(Serialize, Clone)]
pub struct TopProcesses {
    pub cpu: Vec<ProcessEntry>,
    pub mem: Vec<ProcessEntry>,
}

pub struct MonitorState {
    system: System,
    networks: Networks,
    selected_iface: String,
    prev_rx: HashMap<String, u64>,
    prev_tx: HashMap<String, u64>,
}

impl MonitorState {
    pub fn new() -> Self {
        let mut networks = Networks::new_with_refreshed_list();
        networks.refresh(true);

        // Pick the first interface whose name contains "wi" / "wlan" / "wifi", else first non-loopback
        let selected = pick_default_iface(&networks);

        let mut prev_rx = HashMap::new();
        let mut prev_tx = HashMap::new();
        for (name, data) in networks.iter() {
            prev_rx.insert(name.clone(), data.total_received());
            prev_tx.insert(name.clone(), data.total_transmitted());
        }

        Self {
            system: System::new_all(),
            networks,
            selected_iface: selected,
            prev_rx,
            prev_tx,
        }
    }

    pub fn refresh_net(&mut self) -> NetStats {
        self.networks.refresh(true);

        let ifaces: Vec<String> = self.networks.iter().map(|(n, _)| n.clone()).collect();

        let (upload_bps, download_bps) = self
            .networks
            .iter()
            .find(|(name, _)| *name == &self.selected_iface)
            .map(|(name, data)| {
                let rx = data.total_received();
                let tx = data.total_transmitted();
                let prev_rx = *self.prev_rx.get(name).unwrap_or(&rx);
                let prev_tx = *self.prev_tx.get(name).unwrap_or(&tx);
                self.prev_rx.insert(name.clone(), rx);
                self.prev_tx.insert(name.clone(), tx);
                let down = rx.saturating_sub(prev_rx);
                let up = tx.saturating_sub(prev_tx);
                (up, down)
            })
            .unwrap_or((0, 0));

        NetStats {
            interface: self.selected_iface.clone(),
            upload_bps,
            download_bps,
            interfaces: ifaces,
        }
    }

    pub fn refresh_processes(&mut self) -> TopProcesses {
        self.system.refresh_all();

        let total_mem = self.system.total_memory().max(1);

        let mut cpu_list: Vec<ProcessEntry> = self
            .system
            .processes()
            .values()
            .map(|p| ProcessEntry {
                name: p.name().to_string_lossy().into_owned(),
                value: p.cpu_usage(),
            })
            .collect();
        cpu_list.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
        cpu_list.truncate(3);

        // Aggregate same-name processes for memory
        let mut mem_map: HashMap<String, u64> = HashMap::new();
        for p in self.system.processes().values() {
            *mem_map
                .entry(p.name().to_string_lossy().into_owned())
                .or_default() += p.memory();
        }
        let mut mem_list: Vec<ProcessEntry> = mem_map
            .into_iter()
            .map(|(name, mem)| ProcessEntry {
                name,
                value: mem as f32 / total_mem as f32 * 100.0,
            })
            .collect();
        mem_list.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
        mem_list.truncate(3);

        TopProcesses {
            cpu: cpu_list,
            mem: mem_list,
        }
    }

    pub fn set_interface(&mut self, name: String) {
        self.selected_iface = name;
    }

    pub fn interfaces(&self) -> Vec<String> {
        self.networks.iter().map(|(n, _)| n.clone()).collect()
    }
}

fn pick_default_iface(networks: &Networks) -> String {
    let wifi_keywords = ["wi-fi", "wifi", "wlan", "wireless", "wi_fi"];
    for (name, _) in networks.iter() {
        let lower = name.to_lowercase();
        if wifi_keywords.iter().any(|kw| lower.contains(kw)) {
            return name.clone();
        }
    }
    // Fallback: first non-loopback
    for (name, _) in networks.iter() {
        if name != "lo" && name != "Loopback Pseudo-Interface 1" {
            return name.clone();
        }
    }
    networks
        .iter()
        .next()
        .map(|(n, _)| n.clone())
        .unwrap_or_default()
}

pub type SharedState = Mutex<MonitorState>;
