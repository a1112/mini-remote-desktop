# Rdesk Mobile（Android）

独立 Android 应用，支持手机控制 Windows 桌面，以及经手机端授权后由电脑控制手机。媒体链路使用 TLS WebSocket，并要求网关配对令牌和通过局域网发现的证书指纹。

## 编译与安装

需要 Android SDK 36、Java 17 和 Gradle 8.13。项目入口是 `apps/Rdesk-Mobile`。

```powershell
cd apps/Rdesk-Mobile
$env:ANDROID_HOME = 'C:\Users\10428\AppData\Local\Android\Sdk'
gradle :app:testDebugUnitTest :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

安装需要在小米手机开发者选项中允许 USB 安装，并在手机上确认系统弹窗。APK 的包名为 `com.a1112.rdeskmobile`。

## 启动电脑网关

电脑网关必须运行在当前登录用户的交互式 Windows 会话中，否则 Windows 服务会话无法采集桌面。先执行 `cargo build -p mrd-mobile-gateway-app`，再在 PowerShell 中启动：

```powershell
$env:MRD_MOBILE_GATEWAY_BIND = '0.0.0.0:9534'
$env:MRD_MOBILE_GATEWAY_PAIRING_TOKEN = '<至少 32 字节的随机令牌>'
./apps/mrd-mobile-gateway/start.ps1 -ListenAddress '0.0.0.0:9534'
```

手机打开时会广播探测，并探测手机当前网段及常见的 `192.168.0.x`、`192.168.1.x` 网段；点“扫描局域网电脑”可重新搜索。这支持这些网段之间可路由但广播隔离的家庭网络。点选发现的电脑后，在应用中输入与网关相同的配对令牌；应用会固定发现响应中的证书指纹。网关默认只监听 `127.0.0.1:9534`；仅在可信局域网内需要手机直接连接时才设置 `0.0.0.0:9534`，并按需在 Windows 防火墙的专用网络配置放行 TCP `9534` 与 UDP `9535`。其他网段或路由器隔离单播时，可手动输入电脑的私有 IPv4 地址和端口。

## 手机控制电脑

在手机端选择“连接电脑屏幕”。收到画面后，单指触控映射为鼠标左键拖动；工具栏支持滚轮、英文与数字输入、回车和退格。结束时点“断开”。网关会释放仍按下的鼠标和按键。

## 电脑控制手机

先在手机上打开“开启辅助功能设置”，手动启用 Rdesk 移动版。回到应用，点“批准并共享手机屏幕”，在系统弹窗中批准本次采集。然后在电脑浏览器打开 `https://127.0.0.1:9534/mobile/phone?token=<配对令牌>`；浏览器首次访问临时证书时，需要确认该证书的指纹与发现响应中的 `tls_sha256` 一致。浏览器支持点击、滑动、返回、主页和向当前输入框发送文字。手机通知栏和应用内均可立即停止共享。

Android 每次新建屏幕采集会话都要求用户再次批准。辅助功能权限不能通过 ADB 静默授予。屏幕锁定或撤销投屏时，手机端会结束共享。

## 已知限制

- 网关默认使用 TLS WebSocket；启动时必须设置至少 32 字节的 `MRD_MOBILE_GATEWAY_PAIRING_TOKEN`。Android 端连接前输入令牌，并使用发现响应中的 `tls_sha256` 指纹固定网关证书。无 WAN 中继、端到端加密或音频。
- 当前 Android 安装版仍使用 JPEG，代码上限约 6 帧/秒，实际帧率还受 CPU 编解码影响，不能标称 60 FPS。电脑端新 H.264 链路已通过本机约 60 FPS 验证，Android 控制端还需接入 MediaCodec；手机被控端也还需改为 Surface 硬件编码。
- 手机到 Windows 的文字输入通过 Unicode 输入事件发送；目标程序可能自行限制输入。手机作为被控端时，文字输入依赖 Android 当前焦点控件支持辅助功能 `ACTION_SET_TEXT`。
- Windows 安全桌面、提权弹窗等受系统隔离的界面不接受普通 `SendInput`。

## 电脑视频链路本机验证

启动网关后打开 `https://127.0.0.1:9534/mobile/desktop?token=<配对令牌>`，点击“开始 30 秒测试”。页面分别统计实际接收、解码和绘制帧率，以及同机采集到绘制时间。首次访问临时证书时，需要核对发现响应中的 `tls_sha256`。新端点为 `/mobile/desktop/video/ws`，使用 DXGI 共享纹理、NVENC H.264、WebCodecs；需要 NVIDIA NVENC 和支持 WebCodecs 的本机浏览器。启动脚本会为当前进程查找已安装 NVIDIA 驱动目录，不修改系统 PATH。

动态测试、协议及结果见 [本机测试说明](../../tests/mobile-gateway/README.md)。本机测试结果不能当作手机 Wi-Fi 或手机被控性能。

## 启动器图标检查

原生 Android 图标位于 `app/src/main/res`，Manifest 引用 `@mipmap/ic_launcher` 与 `@mipmap/ic_launcher_round`。包含五种密度的 legacy PNG、API26 adaptive 前景/背景和 API33 单色主题层。母版与可复现命令见 [branding/README.md](branding/README.md)。

静态检查不会验证最终 APK。已安装且接受许可的 SDK 36 环境可运行 `bash scripts/build_and_verify_launcher.sh`，检查单元测试、merged manifest 与最终 APK 资源。当前 B optical-v2 母版仍为候选；桌面 Tauri 资源未在本次 Android 修复中更改。
