const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;
const appWindow = getCurrentWindow();

// ── State ──
let currentIface = '';
let allIfaces = [];
let hovering = false;
let latestInfo = { available: true, interface: '', procs: [] };

// ── DOM refs ──
const widget   = document.getElementById('widget');
const upVal    = document.getElementById('up-val');
const upUnit   = document.getElementById('up-unit');
const downVal  = document.getElementById('down-val');
const downUnit = document.getElementById('down-unit');

// ── Theme (persisted; toggled via right-click) ──
widget.dataset.theme = localStorage.getItem('theme') || 'dark';
widget.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  const next = widget.dataset.theme === 'dark' ? 'light' : 'dark';
  widget.dataset.theme = next;
  localStorage.setItem('theme', next);
});

// ── Left-drag to move; middle-click cycles network interface ──
widget.addEventListener('mousedown', (e) => {
  if (e.button === 0) appWindow.startDragging();
  if (e.button === 1) e.preventDefault(); // suppress autoscroll
});
widget.addEventListener('auxclick', async (e) => {
  if (e.button !== 1 || allIfaces.length < 2) return;
  const idx = allIfaces.indexOf(currentIface);
  const next = allIfaces[(idx + 1) % allIfaces.length];
  await invoke('set_interface', { name: next });
  currentIface = next;
});

// ── Keep the native tooltip stable while hovered (re-assigning title resets
// WebView2's show timer, so we only set it on enter / when not hovering) ──
widget.addEventListener('mouseenter', () => {
  hovering = true;
  widget.title = buildTooltip(latestInfo);
});
widget.addEventListener('mouseleave', () => {
  hovering = false;
});

// ── Formatting ──
function fmtSpeed(bps) {
  const kb = bps / 1024;
  if (kb < 1000) return { v: kb.toFixed(1), u: 'KB/s' };
  const mb = kb / 1024;
  if (mb < 1000) return { v: mb.toFixed(1), u: 'MB/s' };
  return { v: (mb / 1024).toFixed(1), u: 'GB/s' };
}

// Compact form for the tooltip (e.g. 1.2M, 64K, 0)
function fmtShort(bps) {
  if (bps >= 1_048_576) return (bps / 1_048_576).toFixed(1) + 'M';
  if (bps >= 1024) return Math.round(bps / 1024) + 'K';
  return bps + 'B';
}

// ── Tooltip text (native OS toast via title attribute) ──
function buildTooltip(info) {
  const head = `网卡: ${info.interface}` + (allIfaces.length > 1 ? '  (中键切换)' : '');
  if (!info.available) {
    return head + '\n\n进程网速需以管理员身份运行 trafmon';
  }
  if (!info.procs.length) {
    return head + '\n\n— 暂无进程网络流量 —';
  }
  const lines = info.procs.map(
    (p) => `${p.name}  ↓${fmtShort(p.down_bps)} ↑${fmtShort(p.up_bps)}`
  );
  return [head, '────────────', ...lines].join('\n');
}

// ── Poll network speed every second ──
async function pollNet() {
  try {
    const stats = await invoke('get_net_stats');
    const up = fmtSpeed(stats.upload_bps);
    const down = fmtSpeed(stats.download_bps);
    upVal.textContent = up.v;
    upUnit.textContent = up.u;
    downVal.textContent = down.v;
    downUnit.textContent = down.u;

    currentIface = stats.interface;
    allIfaces = stats.interfaces;
  } catch (e) {
    console.error('net poll error', e);
  }
  setTimeout(pollNet, 1000);
}

// ── Poll per-process net speed for the tooltip every 2 seconds ──
async function pollProcs() {
  try {
    latestInfo = await invoke('get_net_processes');
    if (!hovering) widget.title = buildTooltip(latestInfo);
  } catch (e) {
    console.error('proc poll error', e);
  }
  setTimeout(pollProcs, 2000);
}

pollNet();
pollProcs();
