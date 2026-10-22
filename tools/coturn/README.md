# coturn allocation 指标冷启动补丁

本目录保存固定上游提交的最小补丁和公开隔离复现工具，供代码审阅与构建验收。补丁在 allocation worker 启动前注册 Prometheus 指标，并用 checked `prom_gauge_add(..., 0, labels)` 创建配置启用的 allocation 系列。HTTP exporter 的监听仍保持原顺序；scrape 不重置计数，缺失样本不被当作零。

- 上游：官方 [coturn/coturn](https://github.com/coturn/coturn/tree/de0c9b28f22281a0251d7e98aa8a097895a5b185)，commit `de0c9b28f22281a0251d7e98aa8a097895a5b185`。
- 补丁：[allocation-cold-start.patch](allocation-cold-start.patch)，SHA256 `9d5d75e01088952e8f065a890d1e8410ef1af60d25e9806c3e6a8d70648e6bcb`。
- 来源与修改前后摘要：[SOURCE-PINS.json](SOURCE-PINS.json)；上游许可证：[UPSTREAM-LICENSE](UPSTREAM-LICENSE)。
- 复现步骤：[REPRODUCE.md](REPRODUCE.md)。工具只生成指定隔离目录中的构建和 loopback fixtures，不包含服务安装命令。

## 已完成的隔离验收

原始源码和补丁源码均完成 CMake Release 全量构建。原始服务冷启动真实 HTTP /metrics 返回 200，但 allocation 指标无样本；补丁服务为启用的 UDP、TCP、TLS/TCP 返回数值零。

真实 UDP、TCP、TLS/TCP 客户端分别经过 401 challenge 和认证 Allocate，校验事务 ID、方法与 MESSAGE-INTEGRITY，三个系列各增长至 1。普通 Refresh lifetime 600 不重复增长；认证 Refresh lifetime 0 得到成功确认后，在客户端 socket 关闭前观察对应系列回零。三种连接曾同时存在，释放后的全零不依赖超时清理。

启用矩阵另验证 UDP-only、TCP-only 的冷样本集合，以及 DTLS 启用时四类冷样本。**未执行 DTLS Allocate 或 DTLS 生命周期测试。** 原生 pthread fixture 直接编译官方 prom.c，20000 次增长与 20000 次 add0 并发后保留 20001，证明已有非零值不被 add0 清除。

## 构建功能与比较边界

验收使用 GCC 15.2.0、CMake 4.2.3、OpenSSL 3.5.5、libevent 2.1.12、libmicrohttpd 1.0.1；SQL 后端启用 SQLite、hiredis/hiredis_ssl，未启用 PostgreSQL、MySQL、MongoDB、systemd 或 SCTP。原构建参数是 Release、WITH_MYSQL=OFF、BUILD_TESTING=OFF，其余可选依赖按当时主机探测。复现指南显式关闭 PostgreSQL、MongoDB、systemd 自动探测，以保持此功能范围；不同工具链无需产生相同二进制摘要。

被比较的既有安装二进制源码 commit 未知。两者 DT_NEEDED SONAME 集合一致；随后在被审核主机的清理环境中实际核对了 24 个已解析库路径及文件 SHA256，映射一致、未知项为零。既有二进制 RUNPATH 为 /usr/local/lib，隔离候选无 RUNPATH。库映射一致不能证明源码、编译功能或运行行为相同，也不能代替目标环境的独立验收。

本目录只有公开源码工具、补丁与说明。运行产生的 REST secret、认证凭据、配置、临时证书私钥和完整运行证据必须留在 checkout 外的 root 私有隔离目录；不要加入版本控制。
