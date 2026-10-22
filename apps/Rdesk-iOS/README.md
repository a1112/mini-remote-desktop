# Rdesk for iOS

原生 SwiftUI 客户端，支持 iOS / iPadOS 16 及以上。在受信局域网内，通过现有 `mrd-mobile-gateway` 连接 Windows 桌面。无需运行 Vite、Tauri 开发服务器或手机上的本地服务。

## 功能与边界

- 手动输入电脑 IPv4 和端口；通过 UDP 单播搜索 Wi-Fi 当前地址所在的 `/24` 网段，也探测已填写的地址。跨网段连接请手动输入地址。
- 接收 `/mobile/desktop/ws` 的 JPEG 画面，显示原始桌面分辨率及实际接收帧率。
- 轻触点击、按住拖动、上下滚轮、Unicode 文字、回车、退格和 Esc；触控坐标排除画面周围的黑边。
- 切换后台、网络中断或长时间未收到画面时断开，网关在连接关闭后释放鼠标和键盘。发送队列有界，连续移动会合并，按下和抬起保持顺序。
- 记住上次连接地址；适配 iPhone、iPad 和横屏。

目前桌面网关的采集端只支持 Windows。此版本没有 iPhone 屏幕共享/被控、音频、文件传输、公网中继，也未接入 H.264 视频端点。JPEG 网关约每 150 ms 采集一次，不能标称 60 FPS。

**沿用现有网关的信任模型：明文 WebSocket，没有配对口令。只在受信局域网使用，不要将端口映射到公网。** 客户端仅接受 RFC1918 私有 IPv4 和回环地址，拒绝 HTTP 重定向。

## 构建

需要完整 Xcode 和 iOS SDK。仓库已包含 `RdeskMobile.xcodeproj`，普通构建不需要 XcodeGen、Rust、Node 或第三方 Swift 包。

```bash
cd apps/Rdesk-iOS
./scripts/build.sh simulator

# Apple 账号需要已登录 Xcode；填写自己的 Team ID。
APPLE_DEVELOPMENT_TEAM=YOUR_TEAM_ID ./scripts/build.sh device
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
./apps/mrd-mobile-gateway/start.ps1 -ListenAddress '0.0.0.0:9534'
```

在专用网络防火墙中按需放行 TCP `9534` 和 UDP `9535`。手机连接同一局域网，打开 Rdesk，允许本地网络访问，搜索电脑或输入其私有 IPv4 地址。第一次搜索可能受到授权弹窗影响，授权后重新搜索即可。拒绝后可到 iOS 设置中的 Rdesk 权限页重新开启。

局域网权限通过 `NSLocalNetworkUsageDescription` 说明；采用单播发现，无需申请受限的 multicast entitlement。详见 [Apple 本地网络隐私说明](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)。

## 验证

```bash
xcrun simctl list devices available
SIMULATOR_ID=YOUR_SIMULATOR_UDID ./scripts/test.sh
```

测试脚本用 Python 标准库启动仅监听 `127.0.0.1:19534` 的协议夹具，结束时关闭。夹具使用应用图标生成 JPEG，不采集真实桌面，也不注入操作。测试包括地址校验、消息与大小边界、触控黑边映射，以及实际 WebSocket 的 JPEG 解码、点击、按键顺序、文字发送和退出会话。只有 Debug 构建支持 `--gateway-fixture` 测试启动参数。

脚本保留 XCTest 结果、截图和覆盖率，关闭额外的系统诊断收集，避免 Xcode 27 的 `simctl diagnose` 在测试结束后长时间等待。

2026-09-29 本机验证：Xcode 27.0 完成模拟器和 arm64 真机 Release 构建；iPhone 17 / iOS 26.5 模拟器上 5 项单元测试、2 项 UI / WebSocket 测试全部通过（7 passed / 0 failed）。已在配对的 iPhone 16 Pro Max（iOS 27.0.1）安装并启动，核对真机首页。协议夹具测试与真机启动不能替代 iPhone ↔ Windows 的真实局域网画面、操作和性能验收，该端到端验收尚未完成。
