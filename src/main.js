const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;
const { listen } = window.__TAURI__.event;
const appWindow = getCurrentWindow();

// ── State ──
let currentIface = '';
let allIfaces = [];

// ── DOM refs ──
const widget   = document.getElementById('widget');
const upVal    = document.getElementById('up-val');
const upUnit   = document.getElementById('up-unit');
const downVal  = document.getElementById('down-val');
const downUnit = document.getElementById('down-unit');

// ── Theme (persisted; toggled via right-click or tray) ──
widget.dataset.theme = localStorage.getItem('theme') || 'dark';
function toggleTheme() {
  const next = widget.dataset.theme === 'dark' ? 'light' : 'dark';
  widget.dataset.theme = next;
  localStorage.setItem('theme', next);
}
widget.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  toggleTheme();
});

// ── Opacity (persisted; set from tray submenu) ──
function setOpacity(v) {
  widget.style.opacity = String(v);
  localStorage.setItem('opacity', String(v));
}
setOpacity(parseFloat(localStorage.getItem('opacity')) || 1);

// ── Language (persisted; default Chinese). Sync the tray labels on startup to
// match the saved preference, since the tray is built in Chinese by default. ──
const savedLang = localStorage.getItem('lang') || 'zh';
invoke('set_language', { lang: savedLang }).catch(() => {});

// ── Tray -> frontend events ──
listen('toggle-theme', () => toggleTheme());
listen('set-opacity', (e) => setOpacity(e.payload));
listen('set-lang', (e) => localStorage.setItem('lang', e.payload));

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

// ── Hover shows the live detail popup (a separate window, positioned by Rust) ──
widget.addEventListener('mouseenter', () => invoke('show_detail'));
widget.addEventListener('mouseleave', () => invoke('hide_detail'));

// ── Formatting ──
function fmtSpeed(bps) {
  const kb = bps / 1024;
  if (kb < 1000) return { v: kb.toFixed(1), u: 'KB/s' };
  const mb = kb / 1024;
  if (mb < 1000) return { v: mb.toFixed(1), u: 'MB/s' };
  return { v: (mb / 1024).toFixed(1), u: 'GB/s' };
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

pollNet();
