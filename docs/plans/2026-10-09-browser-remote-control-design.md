# 独立网页远程控制设计

状态：用户于 2026-10-09 批准推荐方案。控制电脑无需安装 Rdesk 或 mrd-service；远端 Windows/macOS 运行现有驻留服务。保留上一轮已经验证的设置与后台命令修复。

## 目标与边界

用户在网页登录、选择账户可访问的远端设备、请求有人值守会话。远端同意后，浏览器通过正式 WebRTC 接收真实 H.264 视频，并在获批 scope 内发送指针和键盘。仅在权威授权有效且真实首帧显示后呈现正在观看。拒绝、到期、撤销、注销、页面关闭与断线终止本次连接并释放输入。

首期不实现音频、文件传输、无人值守访问、远程应用和网页被控端。未支持的功能保持明确禁用。浏览器能力不满足时报告原因，不以本机预览、同页 PeerConnection 回环或合成画面替代远程成功。

## 当前缺口与方案选择

网页 launcher 明确拒绝真实远端，通用 web bridge 禁止远程授权与键鼠。已有 preview 直接采集本机 Windows 屏幕；mobile gateway 是独立 LAN Windows/Android 入口，未绑定主线 grant。它们均不能直接提供本需求。

已批准采用独立 BrowserController 身份，接入现有正式 v3 WebRTC、目标同意、scope、签名信令和 DirectFirst + TURN。备选的本机服务桥接版改动较少，但仍要求控制电脑安装服务，因此没有采用。

## 身份、授权与后端

复用用户登录与账户/租户约束的设备目录。新增浏览器专用会话 API，创建短期、Controller-only、绑定 user/tenant/target/session/公钥/有效期的浏览器 principal。标识使用符合现有协议的 `browser_<random hex>`；与物理设备 enrollment、机器 Device Bearer 分开。

临时 Ed25519 私钥只在页面内存中，API 不返回机器设备凭据。数据库实现可用受生命周期约束的内部身份记录满足现有 SessionRequest 外键，但必须有独立浏览器认证类型、期限与撤销校验，不允许它从物理设备 API 获得 Agent 权限，也不混入用户设备列表。

浏览器身份必须接通 realtime credential 验证、sidecar 注册、目标端 `/device-sessions` inspect/approve 与 relay-access；仅增加用户房间 API 不算闭环。沿用 v3 签名 grant 和 request commitment；新增主体认证保持现有原生设备路径兼容。每个浏览器 credential 绑定本次 session 和目标，只能转发该会话的 Controller 消息。

推荐统一浏览器入口 `POST /api/v1/browser-sessions`，输入 session_id、目标、临时公钥和获批前请求的 scopes/profile，返回绑定身份、正式会话与短期信令凭据。提供仅创建者可调用的 inspect、relay-access、close/revoke。实现者依据现有 schema 固定字段后同步 TypeScript/Rust 契约，不能默默改成通用管理 API。

## 信令、媒体与输入

浏览器实现真实签名 registration、v3 request、offer/answer/candidate 和控制协议。Rust 的 serde JSON 字段顺序、context bytes、整数和字节数组均由跨语言 golden vectors 固定；不对任意 JSON.stringify 输出盲签。

视频优先使用现有 H.264 RTP 与 `RTCPeerConnection.ontrack`、`video` 渲染。首期使用浏览器可协商的 H.264 profile 和有界分辨率/帧率；不沿用桌面默认 HEVC 高帧率。ICE 使用经本会话授权的 STUN/TURN 配置，先完成可验证候选 manifest，再发送与现有 v3 一致的签名描述。路由和授权证明通过前不发送输入。

兼容既有 `ctrl_rel`/`ctrl_rt`/`bulk`、MRMX framing、grant/peer/key/scope/sequence/epoch。可靠键事件按 FIFO；移动合并、队列有界；按键状态在 blur、visibilitychange、断线和关闭时清理，目标端保留独立租约失效释放机制。只允许当前会话的 pointer/keyboard scope，不复用静态网关 token 直接注入操作系统输入。

## 网页交互与生命周期

新增独立网页远控页面和 browser remote services，避免继续扩充近万行桌面/测试混合显示组件。共用纯坐标、按钮和键码转换逻辑。现有设备连接入口在浏览器分支调用真实会话 API并跳转该页。

页面显示等待远端同意、协商连接、首帧、权限与错误；刷新只恢复服务器认可的本次会话，未持有有效临时私钥时明确需要重新连接。权限拒绝、会话过期、API/信令失败和媒体失败分别可见。取消后丢弃迟到回调并清理 PeerConnection、数据通道、定时器及临时身份；注销取消浏览器会话且不解绑物理设备。

公网网页采用可信 HTTPS/WSS 和精确 Origin/CORS 配置。开发 localhost 仅用于开发验证；不将机器凭据、TURN 密码、完整 SDP或访问 token 输出到日志/URL。

## 验收

先编写并观察失败的权限、签名契约和页面行为测试，再实现。覆盖跨账户/租户、目标/session/key 绑定、Controller-only、凭据到期/撤销、v3 字节互操作、真实授权和首帧门控、输入顺序及释放、关闭/重开与断线清理。

运行 Python 后端测试、Rust 协议/sidecar/服务检查、Vitest、TypeScript 与 Vite 构建。随后用浏览器与独立真实受控服务验证同意→视频→输入；真实 LAN direct、跨 NAT和TURN路径分别记录证据。构建或模拟测试通过不代表真实远控已验收。若当前安装/权限/设备状态妨碍真实验收，保留代码和具体限制，不能绕过安装身份策略。

WebRTC 接收媒体和数据通道依据 [W3C WebRTC 规范](https://www.w3.org/TR/webrtc/)。
