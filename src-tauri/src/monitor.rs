use serde::Serialize;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Mutex;
use sysinfo::{Networks, System};

const WIFI_KEYWORDS: [&str; 5] = ["wi-fi", "wifi", "wlan", "wireless", "wi_fi"];
const LOOPBACK_NAMES: [&str; 2] = ["lo", "Loopback Pseudo-Interface 1"];

#[derive(Serialize, Clone)]
pub struct NetStats {
    pub interface: String,
    pub upload_bps: u64,
    pub download_bps: u64,
    pub interfaces: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
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

        let names: Vec<String> = networks.iter().map(|(n, _)| n.clone()).collect();
        let selected = pick_default_iface(&names).unwrap_or_default();

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

        let interfaces: Vec<String> = self.networks.iter().map(|(n, _)| n.clone()).collect();

        let (upload_bps, download_bps) = self
            .networks
            .iter()
            .find(|(name, _)| *name == &self.selected_iface)
            .map(|(name, data)| {
                let rx = data.total_received();
                let tx = data.total_transmitted();
                let up = bps_delta(*self.prev_tx.get(name).unwrap_or(&tx), tx);
                let down = bps_delta(*self.prev_rx.get(name).unwrap_or(&rx), rx);
                self.prev_rx.insert(name.clone(), rx);
                self.prev_tx.insert(name.clone(), tx);
                (up, down)
            })
            .unwrap_or((0, 0));

        NetStats {
            interface: self.selected_iface.clone(),
            upload_bps,
            download_bps,
            interfaces,
        }
    }

    pub fn refresh_processes(&mut self) -> TopProcesses {
        self.system.refresh_all();

        let total_mem = self.system.total_memory();

        let cpu_entries: Vec<ProcessEntry> = self
            .system
            .processes()
            .values()
            .map(|p| ProcessEntry {
                name: p.name().to_string_lossy().into_owned(),
                value: p.cpu_usage(),
            })
            .collect();

        let mem_pairs: Vec<(String, u64)> = self
            .system
            .processes()
            .values()
            .map(|p| (p.name().to_string_lossy().into_owned(), p.memory()))
            .collect();

        TopProcesses {
            cpu: top_n(cpu_entries, 3),
            mem: top_n(aggregate_mem(&mem_pairs, total_mem), 3),
        }
    }

    pub fn set_interface(&mut self, name: String) {
        self.selected_iface = name;
    }

    pub fn interfaces(&self) -> Vec<String> {
        self.networks.iter().map(|(n, _)| n.clone()).collect()
    }
}

// ── Pure helpers (unit-tested below) ──

/// Bytes transferred since last sample. `saturating_sub` guards counter resets.
fn bps_delta(prev: u64, current: u64) -> u64 {
    current.saturating_sub(prev)
}

/// Pick the default interface: prefer Wi-Fi, then first non-loopback, then first.
fn pick_default_iface(names: &[String]) -> Option<String> {
    if let Some(n) = names.iter().find(|n| {
        let lower = n.to_lowercase();
        WIFI_KEYWORDS.iter().any(|kw| lower.contains(kw))
    }) {
        return Some(n.clone());
    }
    if let Some(n) = names.iter().find(|n| !LOOPBACK_NAMES.contains(&n.as_str())) {
        return Some(n.clone());
    }
    names.first().cloned()
}

/// Sort entries by value descending and keep the top `n`.
fn top_n(mut entries: Vec<ProcessEntry>, n: usize) -> Vec<ProcessEntry> {
    entries.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(Ordering::Equal));
    entries.truncate(n);
    entries
}

/// Sum memory by process name, then express each as a percentage of total memory.
fn aggregate_mem(pairs: &[(String, u64)], total_mem: u64) -> Vec<ProcessEntry> {
    let total = total_mem.max(1);
    let mut map: HashMap<String, u64> = HashMap::new();
    for (name, mem) in pairs {
        *map.entry(name.clone()).or_default() += *mem;
    }
    map.into_iter()
        .map(|(name, mem)| ProcessEntry {
            name,
            value: mem as f32 / total as f32 * 100.0,
        })
        .collect()
}

pub type SharedState = Mutex<MonitorState>;

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn bps_delta_normal() {
        assert_eq!(bps_delta(1000, 1500), 500);
    }

    #[test]
    fn bps_delta_handles_counter_reset() {
        // current < prev (e.g. interface reset) must not underflow
        assert_eq!(bps_delta(2000, 100), 0);
    }

    #[test]
    fn picks_wifi_interface() {
        let names = vec![s("Ethernet"), s("Wi-Fi"), s("lo")];
        assert_eq!(pick_default_iface(&names), Some(s("Wi-Fi")));
    }

    #[test]
    fn picks_wifi_case_insensitive_and_variants() {
        let names = vec![s("eth0"), s("WLAN-Adapter")];
        assert_eq!(pick_default_iface(&names), Some(s("WLAN-Adapter")));
    }

    #[test]
    fn falls_back_to_first_non_loopback() {
        let names = vec![s("lo"), s("Ethernet0")];
        assert_eq!(pick_default_iface(&names), Some(s("Ethernet0")));
    }

    #[test]
    fn falls_back_to_first_when_only_loopback() {
        let names = vec![s("lo")];
        assert_eq!(pick_default_iface(&names), Some(s("lo")));
    }

    #[test]
    fn empty_interface_list_returns_none() {
        assert_eq!(pick_default_iface(&[]), None);
    }

    #[test]
    fn top_n_sorts_desc_and_truncates() {
        let entries = vec![
            ProcessEntry { name: s("a"), value: 1.0 },
            ProcessEntry { name: s("b"), value: 9.0 },
            ProcessEntry { name: s("c"), value: 5.0 },
            ProcessEntry { name: s("d"), value: 3.0 },
        ];
        let top = top_n(entries, 3);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].name, "b");
        assert_eq!(top[1].name, "c");
        assert_eq!(top[2].name, "d");
    }

    #[test]
    fn top_n_handles_fewer_than_n() {
        let entries = vec![ProcessEntry { name: s("x"), value: 2.0 }];
        assert_eq!(top_n(entries, 3).len(), 1);
    }

    #[test]
    fn aggregate_mem_sums_same_name() {
        // 8 GB total; "chrome" appears twice (1GB + 1GB) -> 25%
        let total = 8 * 1024 * 1024 * 1024u64;
        let one_gb = 1024 * 1024 * 1024u64;
        let pairs = vec![
            (s("chrome"), one_gb),
            (s("chrome"), one_gb),
            (s("code"), one_gb),
        ];
        let mut out = aggregate_mem(&pairs, total);
        out.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap());
        assert_eq!(out[0].name, "chrome");
        assert!((out[0].value - 25.0).abs() < 0.01);
        assert!((out[1].value - 12.5).abs() < 0.01);
    }

    #[test]
    fn aggregate_mem_guards_zero_total() {
        let pairs = vec![(s("p"), 100)];
        // must not divide by zero / produce inf
        let out = aggregate_mem(&pairs, 0);
        assert!(out[0].value.is_finite());
    }
}
