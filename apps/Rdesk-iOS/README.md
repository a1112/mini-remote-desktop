# Rdesk for iOS

原生 SwiftUI 客户端，支持 iOS / iPadOS 16 及以上。在受信局域网内，通过现有 `mrd-mobile-gateway` 连接 Windows 桌面。无需运行 Vite、Tauri 开发服务器或手机上的本地服务。

## 功能与边界

- 手动输入电脑 IPv4 和端口，或通过 UDP 单播搜索 Wi-Fi 当前地址所在的 `/24` 网段。选择搜索结果时，先对照 Windows 网关窗口确认 SHA-256 证书指纹。跨网段连接请手动输入地址。
- 接收 `/mobile/desktop/ws` 的 JPEG 画面，显示原始桌面分辨率及实际接收帧率。
- 轻触点击、按住拖动、上下滚轮、Unicode 文字、回车、退格和 Esc；触控坐标排除画面周围的黑边。
- 切换后台、网络中断或长时间未收到画面时断开，网关在连接关闭后释放鼠标和键盘。发送队列有界，连续移动会合并，按下和抬起保持顺序。
- 记住上次连接地址。配对令牌仅保存在应用当前运行期间。适配 iPhone、iPad 和横屏。

目前桌面网关的采集端只支持 Windows。此版本没有 iPhone 屏幕共享/被控、音频、文件传输、公网中继，也未接入 H.264 视频端点。JPEG 网关约每 150 ms 采集一次，不能标称 60 FPS。

客户端使用 TLS WebSocket，核验网关叶证书的 SHA-256 指纹后才向网关发送配对令牌。仅接受 RFC1918 私有 IPv4 和回环地址，拒绝重定向。请在受信局域网内使用，避免将网关端口映射到公网。

## 构建

需要完整 Xcode 和 iOS SDK。仓库已包含 `RdeskMobile.xcodeproj`，普通构建不需要 XcodeGen、Rust、Node 或第三方 Swift 包。

```bash
cd apps/Rdesk-iOS
bash scripts/build.sh simulator

# Apple 账号需要已登录 Xcode；填写自己的 Team ID。
APPLE_DEVELOPMENT_TEAM=YOUR_TEAM_ID bash scripts/build.sh device
```

产物：

- 模拟器：`build/simulator/Build/Products/Debug-iphonesimulator/RdeskMobile.app`
- 真机 Release：`build/device/Build/Products/Release-iphoneos/RdeskMobile.app`
- 开发签名安装包：`build/Rdesk-iOS.ipa`

IPA 仅能安装到当前描述文件包含的设备，受开发签名有效期限制；这不是 App Store 或 TestFlight 分发包。可直接在 Xcode 打开工程，选择自己的开发团队和 iPhone，点击 Run。也可以用命令安装、启动：

```bash
xcrun devicectl list devices
xcrun devicectl device install app --device YOUR_DEVICE_ID \
  build/device/Build/Products/Release-iphoneos/RdeskMobile.app
xcrun devicectl device process launch --device YOUR_DEVICE_ID com.a1112.rdeskmobile
```

修改工程配置时编辑 `project.yml`，再运行 `xcodegen generate`，一并提交生成的工程。不要复用 `apps/Rdesk/src-tauri/gen/apple` 中旧的、未跟踪的生成文件。

## 连接电脑

在 Windows 当前登录用户的桌面会话里启动网关（不要在 Windows 服务会话内启动）：

```powershell
cargo build -p mrd-mobile-gateway-app
$pairingToken = Read-Host '输入已生成的随机配对令牌（至少 32 字节）' -AsSecureString
$env:MRD_MOBILE_GATEWAY_PAIRING_TOKEN = [Net.NetworkCredential]::new('', $pairingToken).Password
./apps/mrd-mobile-gateway/start.ps1 -ListenAddress '0.0.0.0:9534'
```

在专用网络防火墙中按需放行 TCP `9534` 和 UDP `9535`。手机连接同一局域网，打开 Rdesk 并允许本地网络访问。选择搜索结果时，将应用显示的指纹与 Windows 网关窗口中 `certificate_sha256` 的 64 位十六进制值核对。手动连接时，输入电脑的私有 IPv4 地址、端口和窗口显示的指纹。两种方式都需输入启动网关时设置的配对令牌，然后点击「连接电脑屏幕」。第一次搜索可能受到授权弹窗影响，授权后重新搜索即可。也可在 iOS 设置中的 Rdesk 权限页重新开启本地网络权限。

默认网关每次启动生成一张临时证书，重启后按 Windows 网关窗口的最新指纹重新确认。需要在多次启动间延用同一指纹时，可在网关端同时提供 `MRD_MOBILE_GATEWAY_CERT_PEM` 和 `MRD_MOBILE_GATEWAY_KEY_PEM`。这两个变量的值是 PEM 内容。替换证书后，客户端需要核对并使用新指纹。

局域网权限通过 `NSLocalNetworkUsageDescription` 说明；采用单播发现，无需申请受限的 multicast entitlement。详见 [Apple 本地网络隐私说明](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)。

## 验证

```bash
xcrun simctl list devices available
SIMULATOR_ID=YOUR_SIMULATOR_UDID bash scripts/test.sh
```

测试脚本用 OpenSSL 生成临时证书，再用 Python 标准库启动仅监听回环地址的 TLS JPEG 夹具（`19534`）和 XCTest 控制端点（`19535`），结束时关闭。测试覆盖证书指纹核验、错误指纹在 HTTP 升级前被拒绝、配对令牌校验、JPEG 解码、触控、按键顺序和退出会话。夹具用应用图标生成 JPEG，记录应用经 WebSocket 发送的操作。只有 Debug 构建支持 `--gateway-fixture` 测试预填参数。

脚本保留 XCTest 结果、截图和覆盖率，关闭额外的系统诊断收集，避免 Xcode 27 的 `simctl diagnose` 在测试结束后长时间等待。

2026-11-18 在 iPhone 17 / iOS 26.5 模拟器通过 8 项单元测试和 4 项 UI 测试，并完成未签名的 arm64 iPhoneOS Release 构建。2026-09-29 的版本曾在 iPhone 16 Pro Max 上安装并启动。当前 iPhone ↔ Windows 真机验收还需在同一局域网确认 TLS 配对、真实桌面画面、键鼠操作和断开后的输入释放。
