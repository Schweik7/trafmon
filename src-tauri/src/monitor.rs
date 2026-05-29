use crate::netproc::{self, NetProc};
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use sysinfo::{Networks, Pid, ProcessesToUpdate, System};

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
pub struct NetProcEntry {
    pub name: String,
    pub up_bps: u64,
    pub down_bps: u64,
}

#[derive(Serialize, Clone)]
pub struct NetProcInfo {
    pub available: bool,
    pub interface: String,
    pub procs: Vec<NetProcEntry>,
}

pub struct MonitorState {
    system: System,
    networks: Networks,
    selected_iface: String,
    prev_rx: HashMap<String, u64>,
    prev_tx: HashMap<String, u64>,
    netproc: NetProc,
    prev_proc_counts: HashMap<u32, (u64, u64)>,
    last_proc_sample: Instant,
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
            system: System::new(),
            networks,
            selected_iface: selected,
            prev_rx,
            prev_tx,
            netproc: netproc::start(),
            prev_proc_counts: HashMap::new(),
            last_proc_sample: Instant::now(),
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

    pub fn refresh_net_processes(&mut self) -> NetProcInfo {
        if !self.netproc.available() {
            return NetProcInfo {
                available: false,
                interface: self.selected_iface.clone(),
                procs: Vec::new(),
            };
        }

        let now = Instant::now();
        let dt = (now - self.last_proc_sample).as_secs_f64().max(0.001);
        self.last_proc_sample = now;

        let snapshot = self.netproc.counts_snapshot();

        // Need fresh PID -> name mapping for whatever PIDs are active.
        self.system
            .refresh_processes(ProcessesToUpdate::All, true);

        let mut entries: Vec<NetProcEntry> = Vec::new();
        for (&pid, &(sent, recv)) in &snapshot {
            let (psent, precv) = self.prev_proc_counts.get(&pid).copied().unwrap_or((sent, recv));
            let up = (sent.saturating_sub(psent) as f64 / dt) as u64;
            let down = (recv.saturating_sub(precv) as f64 / dt) as u64;
            if up == 0 && down == 0 {
                continue;
            }
            let name = self
                .system
                .process(Pid::from_u32(pid))
                .map(|p| p.name().to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("PID {pid}"));
            entries.push(NetProcEntry { name, up_bps: up, down_bps: down });
        }

        self.prev_proc_counts = snapshot;

        NetProcInfo {
            available: true,
            interface: self.selected_iface.clone(),
            procs: top_net_procs(entries, 5),
        }
    }

    pub fn set_interface(&mut self, name: String) {
        self.selected_iface = name;
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

/// Sort by combined throughput descending and keep the top `n`.
fn top_net_procs(mut entries: Vec<NetProcEntry>, n: usize) -> Vec<NetProcEntry> {
    entries.sort_by_key(|e| Reverse(e.up_bps + e.down_bps));
    entries.truncate(n);
    entries
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
    fn top_net_procs_sorts_by_total_and_truncates() {
        let entries = vec![
            NetProcEntry { name: s("a"), up_bps: 10, down_bps: 0 },
            NetProcEntry { name: s("b"), up_bps: 0, down_bps: 500 },
            NetProcEntry { name: s("c"), up_bps: 100, down_bps: 100 },
        ];
        let top = top_net_procs(entries, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "b"); // 500 total
        assert_eq!(top[1].name, "c"); // 200 total
    }

    #[test]
    fn top_net_procs_handles_fewer_than_n() {
        let entries = vec![NetProcEntry { name: s("x"), up_bps: 1, down_bps: 1 }];
        assert_eq!(top_net_procs(entries, 5).len(), 1);
    }
}
