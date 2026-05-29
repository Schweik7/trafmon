# 架构说明（ARCHITECTURE）

本文面向后续开发者，说明 trafmon 的整体结构、数据流、关键模块与扩展点。

## 1. 总览

trafmon 是一个 Rust + Tauri v2 的 Windows 桌面悬浮窗。

- **后端（Rust）** 负责系统数据采集：网卡吞吐量（`sysinfo`）与每进程网络流量（ETW / `ferrisetw`），并通过 Tauri 命令暴露给前端；同时承载系统托盘与窗口管理。
- **前端（原生 HTML/CSS/JS）** 只负责展示与交互，不含任何框架或打包器，直接由 `frontendDist` (`../src`) 静态加载。

```
┌─────────────────────────── trafmon 进程 ───────────────────────────┐
│                                                                    │
│  Rust 后端                              前端（WebView2）            │
│  ┌────────────┐  invoke 命令   ┌──────────────────────────────┐    │
│  │ lib.rs     │◀──────────────▶│ main 窗口  (index.html)       │    │
│  │  命令/托盘  │                │   main.js  两行网速 + 交互     │    │
│  │            │  emit 事件      │                              │    │
│  │            │──────────────▶ │ detail 窗口 (tooltip.html)    │    │
│  └─────┬──────┘                │   tooltip.js 进程网速详情     │    │
│        │                       └──────────────────────────────┘    │
│  ┌─────▼──────┐  ┌────────────┐                                    │
│  │ monitor.rs │  │ netproc.rs │  ETW 后台线程                       │
│  │ 网卡/速率   │◀─│ 每进程字节 │                                    │
│  └─────┬──────┘  └─────▲──────┘                                    │
│        │ sysinfo        │ ferrisetw (Microsoft-Windows-Kernel-Net)  │
└────────┼───────────────┼───────────────────────────────────────────┘
         ▼               ▼
   网卡收发计数      内核网络事件（PID, size）
```

## 2. 窗口模型

在 `tauri.conf.json` 中定义两个窗口（均无边框、透明、置顶、不进任务栏）：

| label | 页面 | 说明 |
| --- | --- | --- |
| `main` | `index.html` | 128×66 的主控件，常驻显示两行网速 |
| `detail` | `tooltip.html` | 进程网速详情弹窗，初始 `visible:false`、`focus:false`，悬停时由后端定位并显示 |

`detail` 是独立窗口而非主窗口内的 DOM，原因：主窗口尺寸很小，内部 tooltip 会被裁剪；独立窗口可超出主窗口边界、不改变主控件 UI，并能实时刷新（原生 `title` tooltip 做不到上色与实时更新）。

显示/隐藏与定位由后端命令完成：`show_detail` 读取 `main` 的 `outer_position()/outer_size()`，把 `detail` 放在主窗口正下方并 `set_ignore_cursor_events(true)`（点击穿透）后 `show()`；`hide_detail` 隐藏它。前端在主控件 `mouseenter/mouseleave` 时调用这两个命令。

## 3. 后端模块

### `src/lib.rs` —— 入口、命令、托盘
- 在 `run()` 中 `manage(Mutex<MonitorState>)`，注册命令，并在 `setup()` 中构建系统托盘。
- **命令**（`#[tauri::command]`，经 `generate_handler!` 注册）：
  - `get_net_stats() -> NetStats`
  - `get_net_processes() -> NetProcInfo`
  - `set_interface(name)`
  - `show_detail()` / `hide_detail()`
- **托盘菜单**（`setup` 内构建）：显示/隐藏、`网卡` 子菜单（每个网卡一个 `CheckMenuItem`）、`不透明度` 子菜单（100/85/70/50%）、切换主题、退出。
  - `on_menu_event` 按菜单项 id 分发：`quit`→`app.exit`；`toggle`→显示/隐藏 main；`theme`→`emit("toggle-theme")`；`nic:<name>`→`set_interface` 并更新勾选；`opacity:<pct>`→`emit("set-opacity", pct/100)` 并更新勾选。
  - `on_tray_icon_event` 左键→显示 main 窗口。

### `src/monitor.rs` —— 网卡吞吐量与每进程速率
- `MonitorState` 持有：`sysinfo::System`、`Networks`、当前选中网卡、上一轮收发计数、`NetProc` 句柄、上一轮每进程累计计数与采样时间戳。
- `refresh_net()`：刷新网卡，对选中网卡用 `bps_delta(prev, cur)` 算出每秒上/下行字节，返回 `NetStats { interface, upload_bps, download_bps, interfaces }`。
- `refresh_net_processes()`：若 ETW 不可用返回 `available:false`；否则对 `NetProc` 的累计计数取快照，按时间差算每 PID 速率，用 sysinfo 映射 PID→进程名，`top_net_procs(5)` 取前 5，返回 `NetProcInfo { available, interface, procs:[{name, up_bps, down_bps}] }`。
- **纯函数（带单元测试）**：`bps_delta`（`saturating_sub` 防计数器回绕）、`pick_default_iface`（优先 Wi-Fi 关键字，其次非回环，再次第一个）、`top_net_procs`（按上+下行合计降序取前 N）。

### `src/netproc.rs` —— ETW 每进程网络采集
- `NetProc { counts: Arc<Mutex<HashMap<pid,(sent,recv)>>>, available: Arc<AtomicBool> }`。
- `start()` 在专用线程上：构建 `ferrisetw` Provider（`Microsoft-Windows-Kernel-Network`，GUID `7DD42A49-…`），用**进程唯一**的会话名 `Trafmon-NetProc-<pid>`（ETW 会话会在进程被强杀后残留，固定名会撞 `AlreadyExist`）启动 `UserTrace::start_and_process()`；回调里按 `event_id` 判方向（发送 10/26/42/58，接收 11/27/43/59），解析 `size` 与 `PID` 字段累加到 `counts`。线程随后 `park()` 持有 trace 直到进程结束。
- **管理员权限**：开启实时 ETW 会话需管理员；非管理员下 `start_and_process` 返回「拒绝访问」，`available` 保持 false，前端据此提示。

## 4. 前端

- 两个页面共用 `style.css`：内含 `[data-theme=dark|light]` 的 CSS 变量（含 `--up` 蓝 / `--down` 绿），主控件的网格布局，以及详情弹窗样式。
- `main.js`（main 窗口）：每秒 `get_net_stats` 刷新两行；右键切主题并 `listen('toggle-theme')`；开机读取并 `listen('set-opacity')` 应用不透明度（存 `localStorage`）；左键 `startDragging` 拖动；中键切换网卡；`mouseenter/leave` 调 `show_detail/hide_detail`。
- `tooltip.js`（detail 窗口）：每秒 `get_net_processes` 渲染带色进程行（↑ 蓝、↓ 绿），按内容 `setSize` 自适应高度；通过 `localStorage` + 事件与主窗口共享主题/不透明度。
- **跨窗口共享**：两个窗口同源，`localStorage` 共享；主题/不透明度通过后端 `emit` 的全局事件 `toggle-theme` / `set-opacity` 同步到两个窗口。

## 5. IPC 一览

| 方向 | 名称 | 载荷 |
| --- | --- | --- |
| 前端→后端（命令） | `get_net_stats` | → `NetStats` |
| 前端→后端 | `get_net_processes` | → `NetProcInfo` |
| 前端→后端 | `set_interface` | `{ name }` |
| 前端→后端 | `show_detail` / `hide_detail` | — |
| 后端→前端（事件） | `toggle-theme` | — |
| 后端→前端（事件） | `set-opacity` | `f64`（0–1） |

## 6. 权限（capabilities）

`src-tauri/capabilities/default.json` 作用于 `["main","detail"]`，权限：`core:default`、`core:window:allow-start-dragging`、`core:window:allow-set-size`。自定义命令无需额外权限。

## 7. 构建与运行要点

- 开发：`pnpm dev`（= `cargo tauri dev`）。前端为静态文件，改动 HTML/CSS/JS 需重载窗口；改 Rust 会由 watcher 自动重编并重启。
- 测试：`cd src-tauri && cargo test`。
- 发布：`pnpm build`（= `cargo tauri build`），产物在 `src-tauri/target/release/bundle/`。
- **看进程网速需管理员**：以管理员身份启动可执行文件，否则详情窗提示需要权限。
- 注意：运行中的可执行文件会锁定自身（Windows），重新构建/运行前需先退出（托盘 → 退出）。

## 8. 扩展点

- **新增一个采集指标**：在 `monitor.rs` 增加方法 → 在 `lib.rs` 加 `#[tauri::command]` 并注册进 `generate_handler!` → 前端 `invoke` 调用。
- **新增托盘项**：在 `setup()` 里建菜单项，加入 `MenuBuilder`，并在 `on_menu_event` 里按 id 处理；需要前端响应时用 `app.emit` 发事件，前端 `listen`。
- **调整每进程方向判定 / 字段名**：见 `netproc.rs` 的 `SEND_IDS/RECV_IDS` 与 `try_parse("size"/"PID")`；若 Windows 版本字段差异导致采不到数据，从这里改。
- **多显示器 / 弹窗位置**：`show_detail` 目前固定放在主窗口正下方，可在此加入屏幕边界翻转逻辑。
