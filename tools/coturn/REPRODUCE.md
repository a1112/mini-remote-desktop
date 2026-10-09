# 在隔离 Linux 环境复现

补丁固定官方 commit `de0c9b28f22281a0251d7e98aa8a097895a5b185`。归档 SHA256 为 `3d6fa86bb713379e735342ef640b93f64272b79af0318ad70a6a871cff0ccc93`，补丁及三处修改文件的摘要见 [SOURCE-PINS.json](SOURCE-PINS.json)。

需要 root、Python 3.12+（含 cryptography）、GCC/pthread、CMake、pkg-config、curl，以及 OpenSSL、libevent、libmicrohttpd、SQLite、hiredis/hiredis_ssl 开发库。此次已验证的主机使用 Python 3.14；CMake 构建无需 autoconf、automake 或 libtool。不要使用 Python -O，它会移除脚本中的检查断言。

所有步骤在专用隔离环境中执行。先切换到本目录，再在 root shell 创建 checkout 外的新 mode 0700 工作目录；同一工作目录内的源树、fixture 和输出文件都不覆盖既有结果。

```sh
export COTURN_FIXTURE_ROOT="$(mktemp -d /tmp/coturn-allocation-repro.XXXXXXXX)"
chmod 700 "$COTURN_FIXTURE_ROOT"
curl --fail --location --proto '=https' --proto-redir '=https' \
  https://codeload.github.com/coturn/coturn/tar.gz/de0c9b28f22281a0251d7e98aa8a097895a5b185 \
  --output "$COTURN_FIXTURE_ROOT/official-source.tar.gz"
chmod 600 "$COTURN_FIXTURE_ROOT/official-source.tar.gz"
python3 -B safe_extract_source.py
python3 -B apply_isolated_patch.py
```

抽取器校验完整归档摘要，拒绝绝对路径、..、硬链接与特殊成员；官方归档中的 11 个示例/man 符号链接仅报告而不落地。保留原始源树，另建补丁树；应用器验证修改前后摘要和生成补丁与本目录补丁逐字节相等。使用自己生成的测试证书，不使用官方示例密钥。

## 构建与真实 TURN 生命周期

以下命令只构建，不执行 install。原验收构建参数为 Release、WITH_MYSQL=OFF、BUILD_TESTING=OFF；这里额外显式禁用三个可选依赖探测，避免其它主机已安装的库改变功能范围。SQLite、hiredis/hiredis_ssl 必须在 configure 日志中确认已发现。

```sh
cmake -S "$COTURN_FIXTURE_ROOT/source/coturn-de0c9b28f22281a0251d7e98aa8a097895a5b185" \
  -B "$COTURN_FIXTURE_ROOT/build-baseline" -DCMAKE_BUILD_TYPE=Release \
  -DWITH_MYSQL=OFF -DBUILD_TESTING=OFF \
  -DCMAKE_DISABLE_FIND_PACKAGE_PostgreSQL=TRUE \
  -DCMAKE_DISABLE_FIND_PACKAGE_mongo=TRUE \
  -DCMAKE_DISABLE_FIND_PACKAGE_libsystemd=TRUE
cmake --build "$COTURN_FIXTURE_ROOT/build-baseline" --parallel 2
python3 -B turn_lifecycle_fixture.py baseline

cmake -S "$COTURN_FIXTURE_ROOT/source-patched" \
  -B "$COTURN_FIXTURE_ROOT/build-patched" -DCMAKE_BUILD_TYPE=Release \
  -DWITH_MYSQL=OFF -DBUILD_TESTING=OFF \
  -DCMAKE_DISABLE_FIND_PACKAGE_PostgreSQL=TRUE \
  -DCMAKE_DISABLE_FIND_PACKAGE_mongo=TRUE \
  -DCMAKE_DISABLE_FIND_PACKAGE_libsystemd=TRUE
cmake --build "$COTURN_FIXTURE_ROOT/build-patched" --parallel 2
python3 -B turn_lifecycle_fixture.py patched
python3 -B enabled_transport_matrix.py

cc -std=c11 -D_GNU_SOURCE -Wall -Wextra -Werror -pthread \
  -I "$COTURN_FIXTURE_ROOT/source-patched/src/prometheus" \
  prom_add_zero_fixture.c "$COTURN_FIXTURE_ROOT/source-patched/src/prometheus/prom.c" \
  -o "$COTURN_FIXTURE_ROOT/prom_add_zero_fixture"
"$COTURN_FIXTURE_ROOT/prom_add_zero_fixture"
```

baseline fixture 的成功表示观察到预期 RED：冷启动 /metrics HTTP 200 且无 allocation 样本。patched fixture 的成功表示冷样本为零，并完成真实 UDP、TCP、TLS/TCP Allocate、普通 Refresh 600、Refresh 0 释放确认和 socket 关闭前计数回零。enabled_transport_matrix 只验启用类型的冷样本集合，**不提供 DTLS Allocate 验收**。原生 C fixture 使用真实 prom.c 验证 add0 与并发增长。

fixtures 仅绑定 127.0.0.1 的临时端口，排除 22、3478、5349、9443、9530、9641；使用随机的自有 REST secret 和自签名临时证书。敏感文件在 root0700 工作目录内以0600保存，输出 JSON 不含 secret 或私钥。原始 metrics、日志、证书、配置和 JSON 结果都保留在该工作目录，禁止复制进 Git。失败或重新复现时使用另一个全新目录。

## 工具清单

- safe_extract_source.py：固定归档摘要与路径边界检查。
- apply_isolated_patch.py：保留原始源码，生成并校验最小补丁树。
- fixture_paths.py：统一读取 COTURN_FIXTURE_ROOT 并检查 root 私有目录。
- turn_lifecycle_fixture.py：真实 UDP/TCP/TLS/TCP 认证生命周期与指标。
- enabled_transport_matrix.py：启用/禁用类型的冷样本矩阵。
- prom_add_zero_fixture.c：真实 Prometheus 库的 add0/并发 fixture。

公开交付只调整了隔离目录配置、脚本定位、摘要检查和矩阵失败退出状态；原补丁与 C fixture 保持已审核字节。此整理未重新运行构建或 TURN 测试，既有验收范围见 [README.md](README.md)。
