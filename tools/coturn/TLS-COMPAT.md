# coturn TLS 旧配置兼容补丁

本地兼容补丁基于官方 [coturn/coturn](https://github.com/coturn/coturn/tree/de0c9b28f22281a0251d7e98aa8a097895a5b185) 提交 `de0c9b28f22281a0251d7e98aa8a097895a5b185`。在新的隔离源码树中先应用 [allocation 冷启动补丁](allocation-cold-start.patch)，再应用 [TLS 兼容补丁](tls_legacy_disable.patch)。上游归档与隔离构建步骤见 [REPRODUCE.md](REPRODUCE.md)，许可证见 [UPSTREAM-LICENSE](UPSTREAM-LICENSE)。

TLS 补丁只修改 `src/apps/relay/mainrelay.c`，恢复 `no-tlsv1` 和 `no-tlsv1_1` 的布尔选项解析。TLS 按最严格的显式下限设置一次；DTLS 单独设置版本上下限。未提供或显式关闭旧选项时保留默认下限 0；保留既有 `no-tlsv1_2` 的 DTLS 最大版本 1.0 语义。创建 context 或设置请求的版本限制失败时退出，冷启动和证书 reload 共用该路径，`mainrelay.h` ABI 不变。

| 固定材料 | SHA256 |
| --- | --- |
| 官方源码归档 | `3d6fa86bb713379e735342ef640b93f64272b79af0318ad70a6a871cff0ccc93` |
| allocation 补丁 | `9d5d75e01088952e8f065a890d1e8410ef1af60d25e9806c3e6a8d70648e6bcb` |
| TLS 补丁，4792 字节 | `06dca1e857c75e2da080b2211c95bcabbca10a7734ed29077cf50cc2fbc87b8c` |
| allocation 后的 mainrelay.c | `fc52927742b98256400cb8880e187bb63f396e6eccfa5133ba309e1a8deedbc3` |
| 两个补丁后的 mainrelay.c，171479 字节 | `6be65267500301971e395180cc8846e3333a853e17b65cb02f729d72679b571e` |
| 未修改的 mainrelay.h | `8ba4848ff3afd716c7fb8e1b77f56faebb4863ebc335f250d48fa08495755513` |
| 已通过隔离资格的 turnserver，706128 字节，版本 4.17.2 | `4fc6d1cb6f5035b99a35ff6b84295a241a5715d4f44396dc1acdca01aff84546` |

二进制摘要属于当时验收主机的具体构建；不同工具链不能据此直接声称二进制相同。

已有验收记录包含完整 CMake configure/build 退出码 0/0、10 组配置矩阵、120 次真实冷启动/reload 协议探测、40 次 context 发布探测，以及冷启动和 reload 各 4 类失败注入。失败观察器转发真实 OpenSSL setter 后报告失败，证明应用拒绝继续运行的处理。双旧选项开启时，真实服务拒绝 TLS 1.0/1.1 和 DTLS 1.0，并成功握手 TLS 1.2/1.3、DTLS 1.2。

另有冷启动类型化 allocation 零指标和 UDP/TCP/TLS 各自认证 Allocate、Refresh 600、Refresh 0 的共 9 个事件；事务及 MESSAGE-INTEGRITY 已校验，释放计数在关闭 socket 前回零。**未验收 DTLS Allocate 生命周期。** 既有 DTLS 最大版本 1.0 用例的 14 个原始公开 datagram 已完整解析 record、handshake 与分片，实际 ServerHello BODY 为 DTLS 1.0，严格 DTLS 1.2 客户端拒绝；这一最大版本用例不替代旧协议最低版本拒绝测试。

验收补充 manifest SHA256 为 `75032481acb010097ba98e615898a6b4d166074fa2c1f2b00b31aa487af8d9f4`，完整独立审查 SHA256 为 `e83881c0d2910773fe4b726c41b00f633b3d0f1ba41b2120cb8aca9e3dbc88e7`。本次公开整理只复制补丁并核对源码，未重跑构建或上述协议测试。

**生产未安装该候选。** 隔离资格不证明浏览器、Mac 或 WAN 远控已经可用；生产环境的类型化 EnvironmentFiles、实际库解析、服务配置兼容性、证书维护交接和当前空闲进程权限仍须按 [安装设计](../../docs/plans/2026-10-10-coturn-tls-compatible-installer-design.md) 单独验收。公开目录仅保存代码补丁、摘要与说明，配置、证书、凭据和运行日志保留在私有验收目录。
