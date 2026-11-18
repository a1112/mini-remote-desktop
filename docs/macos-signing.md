# macOS 开发签名

在 macOS 上运行 `scripts/packaging/build_macos_client.sh` 可打包 `Rdesk.app` 和内嵌的 `MrdService.app`。脚本使用同一个签名证书分别签名两个 bundle，并保留各自的 bundle identifier。签名身份选择顺序：`--sign-identity IDENTITY`、`MRD_MACOS_SIGNING_IDENTITY`、`APPLE_SIGNING_IDENTITY`、项目根目录的 `.rdesk-macos-signing-identity`，最后回退到 ad-hoc 签名。

本机配置文件只包含 `security find-identity -v -p codesigning` 所显示的 40 位证书 SHA-1 指纹，可带结尾换行。它被 Git 忽略，不包含私钥；私钥继续由 macOS 钥匙串管理。例如，可将本机已有的代码签名证书指纹写入该文件后运行：

```sh
./scripts/packaging/build_macos_client.sh
# 也可临时选择不同身份，或明确使用 ad-hoc 签名：
./scripts/packaging/build_macos_client.sh --sign-identity '-'
```

构建阶段的 `prepare-mrd-service.mjs` 也使用同一选择规则，避免它先将 service 强制签为临时身份。若直接运行 `pnpm tauri build`，要同时设置 `APPLE_SIGNING_IDENTITY`；推荐使用上述打包脚本，以便最终分别验证服务和外层应用签名。

固定签名身份有助于 macOS 在同一证书和 bundle identifier 下识别后续构建，但更换身份后首次访问现有钥匙串项仍可能需要授权。证书到期、撤销或更换后，应更新本地指纹并重新验证。绝不要删除用于加密旧数据的钥匙串项；屏幕录制、辅助功能和其他系统权限仍由 macOS 管理。Apple Development 证书适用于开发用途，不等于应用已公证，也不替代真实设备、重启和凭据持久化验收。

发布时使用 **Developer ID Application** 证书，并预先通过 `xcrun notarytool store-credentials` 在钥匙串中配置公证凭据。主动提交公证时指定该配置名。

```sh
./scripts/packaging/build_macos_client.sh --sign-identity "Developer ID Application: …" --notary-profile RdeskRelease
```

脚本对内外两个 bundle 启用 hardened runtime 和可信时间戳。Apple 接受后会附加并验证公证票据，再生成带 `-notarized` 后缀的 ZIP、SHA-256 和公证结果 JSON。发行验收应安装这个 ZIP 中的应用，检查 Rdesk 启动、后台服务、权限以及实际控制会话。

服务在后台以非交互方式访问钥匙串：当新签名身份尚未获得旧密钥访问权时，它会报告不可用而不会弹出授权窗口。已有保护状态且需要授权时，可在登录用户的前台会话中对实际签名包运行：

```sh
target/release/bundle/macos/Rdesk.app/Contents/Resources/MrdService.app/Contents/MacOS/mrd-service --authorize-keychain
```

确认系统提示中的应用身份后完成授权；该命令只访问已有密钥，不会初始化或重建缺失的密钥。开发构建的裸可执行文件或不同 bundle 可能具有不同身份，不应作为已签名发行包授权成功的证据。
