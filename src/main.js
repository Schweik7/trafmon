const { invoke } = window.__TAURI__.core;
const { getCurrentWindow, LogicalSize } = window.__TAURI__.window;
const appWindow = getCurrentWindow();

// ── Window size presets ──
const COLLAPSED_W = 150;
const COLLAPSED_H = 70;
const EXPANDED_W = 200;

// ── State ──
let currentIface = '';
let allIfaces = [];
let expanded = false;

// ── DOM refs ──
const widget     = document.getElementById('widget');
const uploadEl   = document.getElementById('upload-speed');
const downloadEl = document.getElementById('download-speed');
const cpuList    = document.getElementById('cpu-list');
const memList    = document.getElementById('mem-list');
const ifaceList  = document.getElementById('iface-list');

// ── Theme (persisted; toggled via right-click) ──
widget.dataset.theme = localStorage.getItem('theme') || 'dark';
widget.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  const next = widget.dataset.theme === 'dark' ? 'light' : 'dark';
  widget.dataset.theme = next;
  localStorage.setItem('theme', next);
});

// ── Drag from anywhere (except interface buttons) ──
widget.addEventListener('mousedown', (e) => {
  if (e.button !== 0) return;
  if (e.target.closest('.iface-btn')) return;
  appWindow.startDragging();
});

// ── Hover expand/collapse with window resize ──
async function resizeToContent() {
  const h = Math.ceil(widget.scrollHeight);
  await appWindow.setSize(new LogicalSize(EXPANDED_W, h));
}

widget.addEventListener('mouseenter', async () => {
  expanded = true;
  widget.classList.add('expanded');
  await resizeToContent();
});

widget.addEventListener('mouseleave', async () => {
  expanded = false;
  widget.classList.remove('expanded');
  await appWindow.setSize(new LogicalSize(COLLAPSED_W, COLLAPSED_H));
});

// ── Speed formatter: keeps to ~4 digits, switches unit ──
function fmtSpeed(bps) {
  const kb = bps / 1024;
  if (kb < 1000) return kb.toFixed(1) + ' KB/s';
  const mb = kb / 1024;
  if (mb < 1000) return mb.toFixed(1) + ' MB/s';
  return (mb / 1024).toFixed(1) + ' GB/s';
}

// ── Render process rows ──
function renderProcs(container, entries) {
  container.innerHTML = '';
  for (const e of entries) {
    const pct = Math.min(e.value, 100);
    const row = document.createElement('div');
    row.className = 'proc-row';
    const name = document.createElement('span');
    name.className = 'proc-name';
    name.title = e.name;
    name.textContent = e.name;
    const barWrap = document.createElement('div');
    barWrap.className = 'proc-bar-wrap';
    const bar = document.createElement('div');
    bar.className = 'proc-bar';
    bar.style.width = pct + '%';
    barWrap.appendChild(bar);
    const pctEl = document.createElement('span');
    pctEl.className = 'proc-pct';
    pctEl.textContent = pct.toFixed(1) + '%';
    row.append(name, barWrap, pctEl);
    container.appendChild(row);
  }
}

// ── Render interface switcher ──
function renderIfaces(ifaces, active) {
  ifaceList.innerHTML = '';
  for (const name of ifaces) {
    const btn = document.createElement('button');
    btn.className = 'iface-btn' + (name === active ? ' active' : '');
    btn.textContent = name;
    btn.title = name;
    btn.addEventListener('click', async (ev) => {
      ev.stopPropagation();
      await invoke('set_interface', { name });
      currentIface = name;
      renderIfaces(allIfaces, name);
    });
    ifaceList.appendChild(btn);
  }
}

// ── Poll network every second ──
async function pollNet() {
  try {
    const stats = await invoke('get_net_stats');
    uploadEl.textContent   = fmtSpeed(stats.upload_bps);
    downloadEl.textContent = fmtSpeed(stats.download_bps);

    if (stats.interface !== currentIface || stats.interfaces.length !== allIfaces.length) {
      currentIface = stats.interface;
      allIfaces    = stats.interfaces;
      renderIfaces(allIfaces, currentIface);
      if (expanded) resizeToContent();
    }
  } catch (e) {
    console.error('net poll error', e);
  }
  setTimeout(pollNet, 1000);
}

// ── Poll processes every 2 seconds ──
async function pollProcs() {
  try {
    const procs = await invoke('get_top_processes');
    renderProcs(cpuList, procs.cpu);
    renderProcs(memList, procs.mem);
    if (expanded) resizeToContent();
  } catch (e) {
    console.error('proc poll error', e);
  }
  setTimeout(pollProcs, 2000);
}

pollNet();
pollProcs();
