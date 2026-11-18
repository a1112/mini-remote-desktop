# Rdesk 产品审查 · 2026-11-18

审查覆盖桌面与浏览器控制端、本机服务与会话代理、FastAPI、Rust 信令、中继、iOS、Android 和发布流程。以下结论基于本地 `main` 的 `fb9103cc` 和检查时的工作区。工作区原有的 R-Auth、同账号授权、打包和验收脚本改动也纳入了本地检查。

## 本轮改动

- 桌面首页把发起控制和允许他人控制本机放在并列入口。设备码、临时密码、登记状态和最近连接都有明确位置。首页识别本机设备码，避免误开测试。桌面和手机网页的访客可从最近连接填入设备码，再输入远端密码。
- 顶栏支持快捷键和设备搜索，按名称、设备码、系统或 IP 查询。设备页区分普通远程、局域网 P2P 和本机测试，连接请求逐笔提交，失败在页面内提示。
- 授权弹窗使用中文权限名称。会话等待页区分授权、路由和媒体阶段，拒绝、失败或结束后可立即返回设备列表。原生连接状态与服务健康查询增加 4 秒 IPC 超时，允许下一次刷新恢复。
- 应用页面异常时提供重新加载和返回首页。测试工作台留在原生客户端的次级入口。
- CI 增加临时 PostgreSQL 与实际 Rust 信令二进制的后端集成测试，覆盖默认轻量后端测试会跳过的数据库与跨语言用例。

## 修复进展

| 项目 | 当前工作区 |
| --- | --- |
| iOS 网关配对 | 使用 WSS，校验证书 SHA-256 指纹后提交配对令牌。网关控制台显示公开指纹。通过 12 项 iOS 模拟器测试和 arm64 Release 构建，网关配对及指纹更新另经过真实二进制回环检查。 |
| Windows 设备登记与恢复 | 默认继续自动登记。需要管理员介入时，界面提供提升权限的终端命令。命令核对 Windows 服务管理器报告的进程，服务按每条管道请求的实际令牌检查管理员权限。驻留服务入口通过 Windows GNU 交叉类型检查，临时验证清单将第三方 `scrap` 依赖的构建配置设为 Windows DXGI。［[登记与恢复说明](server-connectivity-configuration.md)］ |
| 信令与 macOS 驻留服务 | 已验证签名和会话身份的迟到授权在对应会话内丢弃。macOS 在指定的公网初始化故障后限次重启。用户主动停止优先于故障恢复。 |
| 局域网和首帧 | 本机服务存活、局域网发现、公网信令和渲染状态分别显示。原生会话和实机报告使用渲染器呈现计数判断首帧，Windows 1080p60 样例任务的名称和产物明确标注为样例。 |
| R-Auth 和 API 就绪 | 已配置的反代可提供单一可信 `X-Real-IP`，登录配额按客户端分配。`/readyz` 检查数据库和信令协议就绪，`/healthz` 保留进程存活语义。后端集成回归 997 项及 13 项子测试通过。［[部署配置](../../apps/Rdesk-Server/README.md)］ |
| 设备分组、文件与加载 | 分组按真实成员及设备码筛选。本机目录可打开、刷新、复制路径和查看属性。取消复制会等待实际写入停止，保留已提交的文件和当时尚未覆盖的旧目标。远端目录和传输按服务实际能力显示。诊断与显示页按需加载，首屏 JS 从约 1.69 MB 降至约 817 kB。1,188 项前端测试和 769 项本机服务库测试通过。 |
| macOS 公证流程 | 可用 Developer ID 和钥匙串公证配置显式提交。脚本验证并附加正式票据后生成发行 ZIP、SHA-256 和公证回执。［[打包说明](../macos-signing.md)］ |

待验收包括 Windows 已安装服务登记和恢复、iPhone 到 Windows 的画面和真实输入、macOS 发布公证与安装、公网多地区故障恢复以及真实 Keycloak 代理部署。远端文件目录与传输还需实现远端提供者。跨信令节点的浏览器会话迁移仍属于未实现的服务能力。

## 首次审查发现

| 优先级 | 影响 | 依据与建议 |
| --- | --- | --- |
| P0 | 当前 iOS 客户端无法通过正式 Windows 移动网关 | iOS 构造明文 WebSocket URL，网关要求 TLS 和配对令牌。现有模拟器用例使用独立明文夹具。应复用网关的证书指纹与配对契约，并在 iPhone 到 Windows 的链路验收画面、键鼠与断开。［[iOS URL 构造](../../apps/Rdesk-iOS/Sources/GatewayProtocol.swift) · [网关入口](../../apps/mrd-mobile-gateway/src/main.rs) · [iOS 测试范围](../../apps/Rdesk-iOS/README.md)］ |
| P1 | Windows 界面可打开手动登记或凭据恢复，但对应 IPC 无法执行 | Tauri 将请求交给管理 IPC，Windows 的管理端点仅允许读取。产品 IPC 也拒绝登记与恢复。应让界面指向受支持的管理员操作路径，并用已安装 Windows 服务验证。［[Tauri 登记命令](../../apps/Rdesk/src-tauri/src/main.rs) · [管理 IPC](../../apps/mrd-service/src/ipc_server/connection.rs) · [产品 IPC 策略](../../apps/mrd-service/src/ipc_server/product.rs)］ |
| P1 | 单个结束的会话可能触发共享信令重连 | 事件映射器会拒绝已结束会话的迟到授权，信令循环目前将该错误作为连接失败。应维持现有授权校验，并单独处理可预期的会话级拒绝。［[事件映射](../../apps/mrd-service/src/signaling/event_mapper.rs) · [信令循环](../../apps/mrd-service/src/signaling/runtime.rs)］ |
| P1 | 公网栈异常后，macOS 客户端可能需要手动恢复 | 公网栈安装失败时驻留服务主动退出，LaunchAgent 未配置自动恢复。需验证异常退出后的重启行为，并保留用户主动停止服务的能力。［[公网服务监控](../../apps/mrd-service/src/public_connection.rs) · [macOS LaunchAgent](../../apps/mrd-service/src/shell/macos.rs)］ |
| P1 | 在线、可控、首帧到达在部分状态中混在一起 | 服务健康只检查安全存储与退出状态。窗口附着会清除 LAN 启动错误，部分 LAN 会话在接收任务启动时就标为 `streaming`。应分别展示发现、权限、信令、媒体和首帧，并用对端真实输入确认可控。［[健康检查](../../apps/mrd-service/src/handlers/telemetry.rs) · [窗口附着](../../apps/mrd-service/src/handlers/shell.rs) · [LAN 媒体入口](../../apps/mrd-service/src/lan_discovery.rs)］ |
| P1 | R-Auth 需要经过真实反向代理验证并发登录与撤销 | 登录限流使用 socket 对端 IP。同一反代连接来源下的用户会共享配额。Rdesk 会话与 Keycloak 会话撤销各有自己的生命周期。［[OIDC 登录入口](../../apps/Rdesk-Server/app/api/v1/oidc.py) · [后端部署说明](../../apps/Rdesk-Server/README.md)］ |
| P1 | 多地区公网和故障恢复缺少当前实机证据 | 信令在线状态与路由在进程内。历史公网部署记录是一台中继和零备份，多地区实机记录为 `INFRA_FAIL`，浏览器会话不支持路由迁移。双节点、不同网络和故障恢复需要独立验收。［[信令进程状态](../../apps/realtime-server/src/lib.rs) · [公网部署记录](2026-10-05-public-connectivity.md) · [多地区验收](multi-region-turn-relay-acceptance.md) · [浏览器边界](../../apps/Rdesk/docs/browser-remote-control.md)］ |
| P2 | 设备分组和部分文件功能仍无法完成界面承诺 | 分组选择未参与设备筛选，远程目录浏览和若干文件菜单动作仍缺少实现。已将设备列表的模拟动作明确标为暂不可用。［[设备筛选与动作](../../apps/Rdesk/src/app/components/DevicesPage.tsx) · [设备文件页](../../apps/Rdesk/src/app/components/DeviceDetailPage.tsx)］ |
| P2 | 桌面首屏和安装发布还有工程欠账 | Vite 构建的主 JS 约 1.69 MB。测试工作台等次级页面可独立加载。Windows 1080p60 CI 使用仓库样例评估策略，真实 GPU 任务独立运行。macOS 构建、签名、公证和安装后会话需要分别留存证据。［[路由入口](../../apps/Rdesk/src/app/routes.ts) · [实机与样例任务](../../.github/workflows/mainline-e2e.yml) · [macOS 打包](../../.github/workflows/macos-package.yml)］ |

## 首轮本地验证

| 范围 | 结果 |
| --- | --- |
| 桌面与浏览器 | TypeScript 检查与 Vite 构建通过，Vitest 1179 项通过。Chrome 使用本地 IPC 夹具检查深浅主题、设备搜索、桌面和手机网页访客重连及 720×480 原生布局。 |
| Rust 本机链路 | `mrd-application`、`mrd-session` 和 `mrd-service` 的库测试 792 项通过，5 项忽略。IPC 测试 80 项通过。arm64 Tauri 源码检查通过，检查时略过了缺少的服务打包资源。 |
| FastAPI 与信令 | 本地 PostgreSQL 17 和真实信令二进制组合运行 990 项后端测试及 13 项子测试，零跳过。单独运行 Rust 信令服务测试 67 项通过。GitHub 托管的新任务尚无运行结果。 |
| iOS 与 Android | iOS 无签名模拟器构建通过。Android 通过 6 项启动图标测试、资源和尺寸检查。当前环境缺少完整 Android 构建工具链，APK 构建和终端验收待执行。 |
| 发布验收 | 当前公网部署、真实 Keycloak 登录、设备跨网络画面与系统输入、会话撤销后的资源释放、移动端真机和签名安装包均待验收。 |
