# 独立网页远程控制

网页控制端不依赖控制电脑上的 Rdesk、mrd-service 或本地 Web Bridge。受控设备运行驻留服务，用户登录后选择账户可访问的设备，目标端同意并批准 `screen.view` 后才显示远端 H.264 画面。键鼠分别要求 `input.pointer`、`input.keyboard`。

## 服务部署与配置

部署配套的 `Rdesk-Server`、`realtime-server`、受控端 `mrd-service` 和 Rdesk 前端构建；仅更新网页静态文件不能提供新的浏览器主体及授权入口。保持既有物理设备凭据与用户数据。

后端启动的可重复迁移增加 `devices.principal_kind` 及 `browser_controllers`，并校验现有约束。生产 PostgreSQL 数据库应按已有数据库升级流程执行及验证；本轮自动回归包含 SQLite 实际 DDL 与 PostgreSQL catalog 契约，未替代生产数据库升级验收。

浏览器入口需要以下现有配置生效：

- `RDESK_SIGNALING_WS_URL`：浏览器可访问的可信 `wss://…/ws` 地址。localhost 开发允许 HTTP 页面访问本地 WS。
- `RDESK_PUBLIC_SIGNAL_SERVER_DEVICE_ID` 和 `RDESK_PUBLIC_SIGNAL_SERVER_KEY_ID`：与 sidecar 实际签名身份一致；key ID 为 raw Ed25519 公钥的 SHA-256 小写十六进制。
- `RDESK_RELAY_DIRECTORY_SIGNING_KEY_ID`：网页使用目录签名公钥的 SHA-256 pin。后端从已配置的目录签名器导出公钥，不向浏览器发送私钥。
- `RDESK_RELAY_DIRECTORY_SIGNING_PRIVATE_KEY`：继续通过既有受保护配置提供，不放入前端环境或静态文件。
- `RDESK_REALTIME_PRESENCE_URL` 和既有私有 presence 鉴权 secret：后端通过同一信任边界调用 sidecar 的 `/internal/identities`，取得当前在线物理目标的已验证公钥。
- `RDESK_CORS_ORIGINS`：精确配置网页 Origin。跨域部署需 API/信令可信证书；前端使用 `VITE_RDESK_SERVER_URL` 指向该 API。

缺少 pin、目标离线或身份无法验证时，API 返回明确错误，不使用首次连接信任、不导出机器 Device token、不从本机预览冒充远端画面。

网页部署在子路径时，构建需传入相同的 Vite base，例如 `VITE_RDESK_SERVER_URL=https://175.178.16.90/rdesk/api/v1 pnpm build --base /apps/rdesk/`。路由使用构建的 `BASE_URL`，静态服务器需将该前缀下的页面请求回退至同一 `index.html`，并直接提供资源文件；原生客户端和根路径开发保持默认 `/`。

## 浏览器链路

`POST /api/v1/browser-sessions` 使用当前用户 JWT、临时公钥、目标及请求权限创建有人值守会话。浏览器身份最多持续十分钟，只能作为 Controller 操作本次 session/target；物理设备接口不接受这种身份。

网页经真实 challenge 和签名 registration 接入 v3 信令，验证远端签名 grant、服务器当前授权、完整签名 ICE manifest 和中继目录，再建立 H.264 recvonly WebRTC。实际首帧、已验证选路和必要控制通道均就绪后才能发送输入。网络采用 DirectFirst，无法直连时使用获批 TURN；强制中继仍要求真实 relay 路径。

键鼠使用既有 signed control envelope 和 MRDF/MRMX；键事件等待真实签名 ACK。指针事务先确认定位，再确认按钮/滚轮，再恢复后续普通移动。发送与接收队列有界；失焦、页面隐藏或关闭时释放输入。目标端独立复查会话授权，注销、撤销、过期或网络验证失败会终止媒体和控制。

页面地址为 `/browser-session/:id`。临时私钥只保留在页面内存且不可导出；刷新后没有该身份时，需要返回设备列表重新连接。网页不自动注册当前电脑，不调用本地服务管理、LAN 发现或设备解绑。

## 首期限制与验证范围

首期提供单画面、鼠标、键盘和全屏。音频、文件传输、远程应用、无人值守、网页被控端及不中断的路径迁移尚未接入。受控端未批准或不支持的输入权限不会启用。

验证分为独立层级：签名/目录/控制字节互操作，用户及设备授权 API，真实 Rust WebSocket 生命周期，标准 RTC 与受控服务 H.264 编解码，网页真实 Edge 布局及能力。生成帧的 RTC 测试及 mock RTC 页面测试不等同于 Chrome 到 Mac 实机远控；跨 NAT、真实 TURN 网络和操作系统权限仍需部署后逐项验收。

网页实现入口：`src/app/services/browserRemoteSessionService.ts`、`browserRemotePeer.ts`、`browserRemoteControl.ts` 和 `components/BrowserRemoteSessionPage.tsx`。设计及实施记录位于仓库 `docs/plans/2026-10-09-browser-remote-control-*.md`。
