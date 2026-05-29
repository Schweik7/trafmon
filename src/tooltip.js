const { invoke } = window.__TAURI__.core;
const { getCurrentWindow, LogicalSize } = window.__TAURI__.window;
const { listen } = window.__TAURI__.event;
const appWindow = getCurrentWindow();

const WIDTH = 230;

const tip  = document.getElementById('tip');
const head = document.getElementById('tip-head');
const list = document.getElementById('tip-list');

// ── Theme + opacity (shared with the main window via localStorage) ──
tip.dataset.theme = localStorage.getItem('theme') || 'dark';
tip.style.opacity = String(parseFloat(localStorage.getItem('opacity')) || 1);
listen('toggle-theme', () => {
  tip.dataset.theme = tip.dataset.theme === 'dark' ? 'light' : 'dark';
});
listen('set-opacity', (e) => { tip.style.opacity = String(e.payload); });

function fmtShort(bps) {
  if (bps >= 1_048_576) return (bps / 1_048_576).toFixed(1) + ' M';
  if (bps >= 1024) return Math.round(bps / 1024) + ' K';
  return bps + ' B';
}

async function resizeToContent() {
  await appWindow.setSize(new LogicalSize(WIDTH, Math.ceil(tip.scrollHeight)));
}

function renderMsg(headText, msg) {
  head.textContent = headText;
  list.innerHTML = '';
  const m = document.createElement('div');
  m.className = 'tip-msg';
  m.textContent = msg;
  list.appendChild(m);
}

function renderProcs(info) {
  head.textContent = `网卡 ${info.interface}`;
  list.innerHTML = '';
  for (const p of info.procs) {
    const row = document.createElement('div');
    row.className = 'tip-row';

    const name = document.createElement('span');
    name.className = 'tip-name';
    name.textContent = p.name;
    name.title = p.name;

    const up = document.createElement('span');
    up.className = 'tip-rate up';
    up.textContent = '↑ ' + fmtShort(p.up_bps);

    const down = document.createElement('span');
    down.className = 'tip-rate down';
    down.textContent = '↓ ' + fmtShort(p.down_bps);

    row.append(name, up, down);
    list.appendChild(row);
  }
}

async function poll() {
  try {
    const info = await invoke('get_net_processes');
    if (!info.available) {
      renderMsg('进程网速', '需以管理员身份运行');
    } else if (!info.procs.length) {
      renderMsg(`网卡 ${info.interface}`, '— 暂无流量 —');
    } else {
      renderProcs(info);
    }
    await resizeToContent();
  } catch (e) {
    console.error('detail poll error', e);
  }
  setTimeout(poll, 1000);
}

poll();
