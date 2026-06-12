//! Per-process network throughput via ETW (Microsoft-Windows-Kernel-Network).
//! Starting a real-time ETW session requires Administrator privileges; when that
//! fails we expose `available() == false` and the UI falls back to a hint.
//!
//! NOTE: do not set `.any()/.level()` on the provider. ETW treats
//! `MatchAnyKeyword == 0` (the default) as "match all", and forcing a non-zero
//! mask suppresses delivery from this provider.

use ferrisetw::parser::Parser;
use ferrisetw::provider::Provider;
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::UserTrace;
use ferrisetw::EventRecord;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Microsoft-Windows-Kernel-Network
const KERNEL_NETWORK_GUID: &str = "7DD42A49-5329-4832-8DFD-43D979153A88";
const SESSION_NAME: &str = "Trafmon-NetProc";

// Event IDs that carry a payload `size` for the owning `PID`.
// TCP: 10/26 send, 11/27 recv (IPv4/IPv6).  UDP: 42/58 send, 43/59 recv.
const SEND_IDS: [u16; 4] = [10, 26, 42, 58];
const RECV_IDS: [u16; 4] = [11, 27, 43, 59];

/// Cumulative (sent, received) bytes per PID since the trace started.
pub type Counts = Arc<Mutex<HashMap<u32, (u64, u64)>>>;

#[derive(Clone)]
pub struct NetProc {
    counts: Counts,
    available: Arc<AtomicBool>,
    /// Total matched events since `start`. Monotonic; the monitor watches its
    /// delta as the ETW heartbeat.
    events_seen: Arc<AtomicU64>,
    /// Tells the owning thread to drop the trace and exit. Used to rebuild a
    /// session that has gone silent.
    stop: Arc<AtomicBool>,
}

impl NetProc {
    pub fn counts_snapshot(&self) -> HashMap<u32, (u64, u64)> {
        self.counts.lock().unwrap().clone()
    }

    pub fn available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    pub fn events_seen(&self) -> u64 {
        self.events_seen.load(Ordering::Relaxed)
    }

    /// Drop counts for PIDs that aren't in `live`. The map otherwise grows
    /// unbounded with every transient process that ever sent a packet, which
    /// after long uptime makes `counts_snapshot` slow enough to back the IPC
    /// pipeline up and freeze the tooltip.
    pub fn retain_pids(&self, live: &HashSet<u32>) {
        let mut map = self.counts.lock().unwrap();
        map.retain(|pid, _| live.contains(pid));
    }

    /// Signal the owning thread to drop the trace and exit. Non-blocking;
    /// the kernel-side session goes away within a few hundred ms.
    pub fn shutdown(&self) {
        self.available.store(false, Ordering::Relaxed);
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Stop any leftover Trafmon ETW sessions. Sessions outlive their creating
/// process, and ETW caps a provider at 8 concurrent enables — so leaked
/// sessions from killed runs eventually block new event delivery. Requires the
/// current process to be elevated (it is when this feature is in use).
fn stop_stale_sessions() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Ok(out) = std::process::Command::new("logman")
        .args(["query", "-ets"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let name = line.split_whitespace().next().unwrap_or("");
        if name.starts_with(SESSION_NAME) {
            let _ = std::process::Command::new("logman")
                .args(["stop", name, "-ets"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
    }
}

/// Start the ETW session on a dedicated thread that owns the trace handle for
/// the lifetime of this `NetProc`. Returns immediately with shared handles.
/// Call `shutdown()` to drop the trace; the kernel session ends within ~250 ms.
pub fn start() -> NetProc {
    let counts: Counts = Arc::new(Mutex::new(HashMap::new()));
    let available = Arc::new(AtomicBool::new(false));
    let events_seen = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    let cb_counts = counts.clone();
    let cb_available = available.clone();
    let cb_events = events_seen.clone();
    let cb_stop = stop.clone();

    std::thread::Builder::new()
        .name("etw-netproc".into())
        .spawn(move || {
            // Clear leaked sessions from previous (killed) runs first, so we
            // stay under ETW's per-provider 8-session enable limit.
            stop_stale_sessions();

            let counts_cb = cb_counts.clone();
            let events_cb = cb_events.clone();
            let provider = Provider::by_guid(KERNEL_NETWORK_GUID)
                .add_callback(move |record: &EventRecord, sl: &SchemaLocator| {
                    let id = record.event_id();
                    let is_send = SEND_IDS.contains(&id);
                    let is_recv = RECV_IDS.contains(&id);
                    if !is_send && !is_recv {
                        return;
                    }
                    let Ok(schema) = sl.event_schema(record) else { return };
                    let parser = Parser::create(record, &schema);
                    let size: u32 = parser.try_parse("size").unwrap_or(0);
                    if size == 0 {
                        return;
                    }
                    let pid: u32 = parser
                        .try_parse("PID")
                        .unwrap_or_else(|_| record.process_id());

                    events_cb.fetch_add(1, Ordering::Relaxed);

                    let mut map = counts_cb.lock().unwrap();
                    let entry = map.entry(pid).or_insert((0, 0));
                    if is_send {
                        entry.0 += size as u64;
                    } else {
                        entry.1 += size as u64;
                    }
                })
                .build();

            // Fixed name; stale instances were stopped above so there is no
            // collision and at most one session ever exists.
            match UserTrace::new()
                .named(SESSION_NAME.to_string())
                .enable(provider)
                .start_and_process()
            {
                Ok(trace) => {
                    cb_available.store(true, Ordering::Relaxed);
                    // Hold `trace` here; ferrisetw processes events on its own
                    // thread. Poll for shutdown so an unhealthy session can be
                    // torn down and replaced.
                    while !cb_stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    cb_available.store(false, Ordering::Relaxed);
                    drop(trace);
                }
                Err(e) => {
                    eprintln!(
                        "[trafmon] ETW network trace unavailable (run as Administrator): {e:?}"
                    );
                }
            }
        })
        .ok();

    NetProc {
        counts,
        available,
        events_seen,
        stop,
    }
}
