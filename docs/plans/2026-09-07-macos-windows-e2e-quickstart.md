# macOS ↔ Windows E2E 快速进入

适用基线：`9d7e5987b83759db65eb815182eb83e9daf9b116`。双方应使用同一提交和实际重新构建的 mrd-service。入口已从当前源码核对，双机通过情况需实测。

## 从界面进入

1. 两台机器连接同一局域网，并都打开 **Rdesk 桌面程序**。
2. 控制端左侧 **测试工作台 → E2E**，找到 **LAN E2E 自动化**。
3. “跨设备场景”选 **发现/配对预检**，点击 **开始跨设备 E2E**，确认报告中的目标设备确实是另一台测试机。这一步只证明发现。
4. 改为 **远程显示 Smoke**，再次开始。被控端出现授权请求时核对请求并同意，观察控制端新开的远程显示窗口和解码/渲染计数。
5. 基础显示成功后，再按需要执行 **安全远程显示**、**输入控制 ACK** 和 **媒体画像校验**。输入测试在专用空白窗口进行。故障恢复预检若提示能力不支持，不能算恢复测试通过。

在 macOS 点击开始即为 macOS 控制 Windows；反向时在 Windows 点击开始。测试工作台顶部的本地测试配置与 LAN E2E 区域是不同入口。当前 LAN E2E 区域没有独立目标设备下拉框；未显式指定 targetDeviceId 时自动选取发现列表中的合适 peer。有多台测试机时使用下面的环境变量指定目标。

首次建议先执行 macOS 控制 Windows 的发现和显示，再反向验证 macOS 采集。只启动 `pnpm dev` 打开的普通浏览器页面不能替代 Tauri 桌面程序的本地 IPC 和原生显示能力。

## macOS 编译启动

在已经同步到相同提交的仓库根目录执行；需要 Xcode Command Line Tools、Rust 和 Node/pnpm：

```bash
rustup toolchain install 1.98.0 --profile minimal
export RUSTUP_TOOLCHAIN=1.98.0
pnpm --dir apps/Rdesk install --frozen-lockfile
pnpm --dir apps/Rdesk tauri:dev
```

`tauri:dev` 会构建 mrd-service、生成/签名 `target/debug/MrdService.app`、启动 Vite 和 Rdesk。保留终端运行。

macOS 系统设置中允许相关 Rdesk/MrdService 进程访问本地网络。作为被控端时，在“隐私与安全性”中给实际采集进程允许屏幕录制；键鼠控制还需要实际输入进程的辅助功能权限。以系统弹窗标识的进程为准，修改权限后按提示重启对应进程。

## Windows 本地启动

本轮采用嵌入前端的 debug 桌面程序，构建命令为：

```powershell
pnpm --dir apps/Rdesk build
cargo +1.98.0 build --locked -p mrd-service -p mrd-session-agent
cargo +1.98.0 build --locked -p app --features custom-protocol
Start-Process -FilePath ./target/debug/app.exe -Verb RunAs
```

Rdesk 会按需要拉起同目录的 mrd-service。启动位置是仓库 `target/debug/app.exe`；这个构建不依赖 Vite 常驻。首次出现防火墙提示时允许测试所用的专用网络通信。

本机首次启动实测发现：当前 Windows 前台服务也校验 `%ProgramData%\MiniRemoteDesktop` 的受保护 ACL，目录缺失会报 `protected product directory verification open failed: 0x80070002`。已用 `target/e2e-startup-20260907/start-protected-service.ps1` 在提权进程中按源码的 bootstrap ACL 创建缺失目录并启动服务；仅 SYSTEM/Administrators 获得权限，已有目录不会被脚本改写。服务日志确认 LAN discovery 和 IPC 已启动。普通权限 IPC 客户端实测返回 Access denied，因此本机采用 **Rdesk 与 mrd-service 都以管理员权限运行** 的开发启动方式。系统提权提示由操作者确认；这不代表普通用户安装模式已通过验收。

## 指定目标并自动进入发现预检

先完全退出当前 Rdesk UI，再启动。把示例 device ID 替换为对端实际 ID；值可从发现报告的目标设备信息取得。此模式仅自动发现，避免启动时直接操作对端桌面。

macOS：

```bash
MRD_LAN_E2E_AUTORUN=1 \
MRD_LAN_E2E_SCENARIO=cross.e2e.discovery \
MRD_LAN_E2E_TARGET_DEVICE_ID='<Windows实际device-id>' \
MRD_LAN_E2E_TRANSPORT=quic \
pnpm --dir apps/Rdesk tauri:dev
```

Windows（在管理员 PowerShell 中执行，以便环境变量直接传给程序）：

```powershell
$env:MRD_LAN_E2E_AUTORUN='1'
$env:MRD_LAN_E2E_SCENARIO='cross.e2e.discovery'
$env:MRD_LAN_E2E_TARGET_DEVICE_ID='<macOS实际device-id>'
$env:MRD_LAN_E2E_TRANSPORT='quic'
Start-Process -FilePath ./target/debug/app.exe
```

启动后进入 `/test/e2e`。注意当前手动“开始”按钮只传场景和自适应配置，不保留 autorun 的目标/profile 参数；多设备场景要固定对端时，退出后把 `MRD_LAN_E2E_SCENARIO` 改为 `cross.e2e.remote_display_smoke` 并重新启动，保留目标 device ID。也支持 `MRD_LAN_E2E_PROFILE_WIDTH=1920`、`MRD_LAN_E2E_PROFILE_HEIGHT=1080`、`MRD_LAN_E2E_PROFILE_FPS=60`、`MRD_LAN_E2E_PROFILE_BITRATE_MBPS=20`、`MRD_LAN_E2E_PROFILE_CODEC=h264`；四项尺寸/帧率/码率必须一起设置才形成指定 profile。先完成 1080p60 基线，再测 2K/高刷。

## 常见失败定位

| 报告或现象 | 检查 |
| --- | --- |
| `service_unhealthy` | 本地服务进程、IPC、是否仍运行旧版本服务 |
| `peer_not_found` | 双方都启动 Rdesk、同一局域网、macOS 本地网络权限、Windows 防火墙；默认发现端口 UDP 21116 |
| `peer_not_ready` / 版本不匹配 | 双方服务提交及 QUIC/编解码能力，重建并重启服务 |
| 授权超时/拒绝 | 被控端授权弹窗与请求 scopes，不跳过授权 |
| 无画面/权限错误 | 实际捕获源、屏幕录制权限、decode 和 present 计数 |
| FPS 不达标 | selected profile、实际屏幕刷新率、软硬件路径；不能把降级行当高刷通过 |
| fault injection 不支持 | 标记 skipped，先完成显示闭环，单列恢复能力缺口 |

LAN 测试不要求先搭建 WAN TURN 实验环境；如路由选择为 WAN 或测试 WAN，则另按主线端测计划配置后端和中继。
