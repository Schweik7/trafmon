const { invoke } = window.__TAURI__.core;

// ── State ──
let currentIface = '';
let allIfaces = [];

// ── DOM refs ──
const uploadEl   = document.getElementById('upload-speed');
const downloadEl = document.getElementById('download-speed');
const cpuList    = document.getElementById('cpu-list');
const memList    = document.getElementById('mem-list');
const ifaceList  = document.getElementById('iface-list');
const ifaceLabel = document.getElementById('iface-label');
const themeBtn   = document.getElementById('theme-btn');
const widget     = document.getElementById('widget');

// ── Theme ──
const savedTheme = localStorage.getItem('theme') || 'dark';
widget.dataset.theme = savedTheme;
themeBtn.addEventListener('click', () => {
  const next = widget.dataset.theme === 'dark' ? 'light' : 'dark';
  widget.dataset.theme = next;
  localStorage.setItem('theme', next);
});

// ── Speed formatter ──
function fmtSpeed(bps) {
  if (bps >= 1_048_576) return (bps / 1_048_576).toFixed(1) + ' MB/s';
  if (bps >= 1024)      return (bps / 1024).toFixed(0) + ' KB/s';
  return bps + ' B/s';
}

// ── Render process rows ──
function renderProcs(container, entries, barClass) {
  container.innerHTML = '';
  for (const e of entries) {
    const pct = Math.min(e.value, 100);
    const row = document.createElement('div');
    row.className = 'proc-row';
    row.innerHTML = `
      <span class="proc-name" title="${e.name}">${e.name}</span>
      <div class="proc-bar-wrap"><div class="proc-bar ${barClass}" style="width:${pct}%"></div></div>
      <span class="proc-pct">${pct.toFixed(1)}%</span>`;
    container.appendChild(row);
  }
}

// ── Render iface buttons ──
function renderIfaces(ifaces, active) {
  ifaceList.innerHTML = '';
  for (const name of ifaces) {
    const btn = document.createElement('button');
    btn.className = 'iface-btn' + (name === active ? ' active' : '');
    btn.textContent = name;
    btn.addEventListener('click', async () => {
      await invoke('set_interface', { name });
      currentIface = name;
      ifaceLabel.textContent = name;
      renderIfaces(allIfaces, name);
    });
    ifaceList.appendChild(btn);
  }
}

// ── Poll net stats every second ──
async function pollNet() {
  try {
    const stats = await invoke('get_net_stats');
    uploadEl.textContent   = fmtSpeed(stats.upload_bps);
    downloadEl.textContent = fmtSpeed(stats.download_bps);

    if (stats.interface !== currentIface || stats.interfaces.length !== allIfaces.length) {
      currentIface = stats.interface;
      allIfaces    = stats.interfaces;
      ifaceLabel.textContent = currentIface;
      renderIfaces(allIfaces, currentIface);
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
    renderProcs(cpuList, procs.cpu, 'cpu-bar');
    renderProcs(memList, procs.mem, 'mem-bar');
  } catch (e) {
    console.error('proc poll error', e);
  }
  setTimeout(pollProcs, 2000);
}

// ── Dragging (fallback JS drag for non-drag-region areas) ──
// Tauri's data-tauri-drag-region handles the titlebar; nothing else needed.

pollNet();
pollProcs();
