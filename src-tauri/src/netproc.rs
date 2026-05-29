//! Per-process network throughput via ETW (Microsoft-Windows-Kernel-Network).
//! Starting a real-time ETW session requires Administrator privileges; when that
//! fails we expose `available() == false` and the UI falls back to a hint.

use ferrisetw::parser::Parser;
use ferrisetw::provider::Provider;
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::UserTrace;
use ferrisetw::EventRecord;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// Microsoft-Windows-Kernel-Network
const KERNEL_NETWORK_GUID: &str = "7DD42A49-5329-4832-8DFD-43D979153A88";

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
}

impl NetProc {
    pub fn counts_snapshot(&self) -> HashMap<u32, (u64, u64)> {
        self.counts.lock().unwrap().clone()
    }

    pub fn available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }
}

/// Start the ETW session on a dedicated, parked thread that owns the trace for
/// the lifetime of the process. Returns immediately with shared handles.
pub fn start() -> NetProc {
    let counts: Counts = Arc::new(Mutex::new(HashMap::new()));
    let available = Arc::new(AtomicBool::new(false));

    let cb_counts = counts.clone();
    let cb_available = available.clone();

    std::thread::Builder::new()
        .name("etw-netproc".into())
        .spawn(move || {
            let provider = Provider::by_guid(KERNEL_NETWORK_GUID)
                // Capture every keyword and level: the Kernel-Network data events
                // carry keywords, so the default mask (0) would filter them all out.
                .any(u64::MAX)
                .level(0xff)
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

                    let mut map = cb_counts.lock().unwrap();
                    let entry = map.entry(pid).or_insert((0, 0));
                    if is_send {
                        entry.0 += size as u64;
                    } else {
                        entry.1 += size as u64;
                    }
                })
                .build();

            // Unique per-process name: ETW sessions outlive their creating
            // process, so a fixed name collides (AlreadyExist) after a crash/kill.
            let session = format!("Trafmon-NetProc-{}", std::process::id());
            match UserTrace::new()
                .named(session)
                .enable(provider)
                .start_and_process()
            {
                Ok(_trace) => {
                    cb_available.store(true, Ordering::Relaxed);
                    // Hold `_trace` for the process lifetime; the actual event
                    // processing runs on ferrisetw's own spawned thread.
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[trafmon] ETW network trace unavailable (run as Administrator): {e:?}"
                    );
                }
            }
        })
        .ok();

    NetProc { counts, available }
}
