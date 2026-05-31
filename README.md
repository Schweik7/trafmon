# trafmon

> **中文** · [English](./README.en.md)

一个小巧、无边框、置顶的 Windows 桌面悬浮窗小控件，一眼查看实时网速。基于
**Rust + Tauri v2** 开发。

```
 ↑  193.9 KB/s
 ↓  199.5 KB/s
```

![trafmon 示意图](./示意图.png)

## 下载安装

### 包管理器一键安装

**Scoop**：

```powershell
scoop bucket add trafmon https://github.com/Schweik7/scoop-bucket
scoop install trafmon
```

**winget**（待 [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) 合并后可用）：

```powershell
winget install Schweik7.trafmon
```

### 手动下载

前往 [Releases](https://github.com/Schweik7/trafmon/releases/latest) 下载，或直接选择：

| 安装包 | 说明 |
| --- | --- |
| [trafmon_0.3.0_x64_en-US.msi](https://github.com/Schweik7/trafmon/releases/download/v0.3.0/trafmon_0.3.0_x64_en-US.msi) | MSI 安装程序（x64） |
| [trafmon_0.3.0_x64-setup.exe](https://github.com/Schweik7/trafmon/releases/download/v0.3.0/trafmon_0.3.0_x64-setup.exe) | NSIS 安装程序（x64） |

> 仅支持 64 位 Windows 10 / 11。需查看每个进程的网速时，请以管理员身份运行（详见下文）。

## 功能

- **紧凑两行显示** —— 上传（↑）和下载（↓）速度，每秒刷新。单位随速度自动切换
  （KB/s → MB/s → GB/s）以保持约 4 位数字；数值右对齐，单位竖直对齐。
- **悬停查看进程网速** —— 鼠标悬停时弹出一个独立的小窗，列出占用带宽最多的进程，
  带颜色区分的每进程 ↑/↓ 速率，**每秒实时刷新**。*（需以管理员身份运行，见下文。）*
- **网卡选择** —— 默认选中 Wi-Fi 网卡；可通过托盘菜单或中键点击切换。
- **日间 / 夜间主题** —— 右键或托盘菜单切换，自动持久化。
- **系统托盘菜单** —— 显示/隐藏小窗、选择网卡、调整窗口不透明度、切换主题、退出。
- **无边框、半透明、可拖动** —— 任意位置拖动，始终置顶，不在任务栏显示。

## 交互方式

| 操作 | 效果 |
| --- | --- |
| 左键拖动（任意位置） | 移动小窗 |
| 鼠标悬停 | 弹出实时进程网速详情窗 |
| 右键 | 切换 日间 / 夜间 主题 |
| 中键 | 切换到下一个网卡 |
| 左键单击托盘图标 | 显示小窗 |
| 右键托盘图标 | 菜单：显示/隐藏 · 网卡 · 不透明度 · 主题 · 关于 · 退出 |

## 管理员权限要求

每个进程的网络速率通过 **ETW**（`Microsoft-Windows-Kernel-Network`）采集。开启实时
ETW 会话需要管理员权限。表现如下：

- **普通运行** —— 两行总网速正常工作；悬停详情窗提示需要管理员权限。
- **以管理员身份运行** —— 详情窗列出每个进程的 ↑/↓ 速率。

## 技术栈

- **Tauri v2** —— 无边框透明窗口、系统托盘、前后端 IPC。
- **[`sysinfo`](https://crates.io/crates/sysinfo)** —— 各网卡吞吐量、PID → 进程名映射。
- **[`ferrisetw`](https://crates.io/crates/ferrisetw)** —— 安全的 ETW 消费端，采集每进程网络字节数。
- 原生 HTML/CSS/JS 前端（无框架、无打包器）。

## 开发

环境要求：[Rust](https://rustup.rs)、带 [pnpm](https://pnpm.io) 的
[Node.js](https://nodejs.org)，以及 Tauri CLI：

```bash
cargo install tauri-cli --version "^2"
```

开发模式运行：

```bash
pnpm install
pnpm dev          # 等价于 cargo tauri dev
```

运行 Rust 单元测试：

```bash
cd src-tauri && cargo test
```

## 构建

```bash
pnpm build        # 等价于 cargo tauri build
```

产物（安装包 / 可执行文件）位于 `src-tauri/target/release/bundle/`。若需查看每进程网速，
请以管理员身份启动生成的可执行文件。

## 项目结构

```
src/                 前端（index.html、style.css、main.js、tooltip.html、tooltip.js）
src-tauri/
  src/lib.rs         Tauri 命令 + 系统托盘
  src/monitor.rs     网卡吞吐量、网卡选择、每进程速率计算
  src/netproc.rs     ETW 每进程网络采集
  tauri.conf.json    窗口配置（无边框、透明、置顶）
```

更详细的模块与数据流说明见 [ARCHITECTURE.md](./ARCHITECTURE.md)。
