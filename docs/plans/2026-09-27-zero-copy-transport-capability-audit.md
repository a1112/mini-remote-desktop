# 本机零拷贝：传输与能力声明审计

日期：2026-09-27。仓库：`L:\project\mini-remote-desktop`。

本报告记录实施前的只读审计结果与建议；审计阶段仅新增此文件，没有修改产品代码，没有运行性能测试。用户随后批准的实施工作另按 `2026-09-27-local-zero-copy-implementation.md` 跟踪。路径和行号按审计时工作区读取结果记录，后续改动可能使行号移动。范围包含主线 `mrd-service`、`mrd-hardware`、`mrd-pipeline-core`、QUIC/WebRTC 传输，以及 Rdesk benchmark 的零拷贝统计。未读取 `junk/`，未处理已有工作区更改。

## 本机证据与定义

主任务报告：本机 NVDEC baseline 以 `failed to load nvcuda.dll` 失败。该结果说明当前执行环境的 NVDEC 链路尚未通过，不等同于证明机器没有 NVIDIA GPU，也不能据此区分驱动缺失、加载路径或其他运行环境问题。子任务没有自行重复该硬件测试。

baseline 命令：`cargo test -p mrd-decode-nvdec --test nvdec_probe -- --nocapture --test-threads=1`。主任务记录初始 11 个测试通过、3 个失败，部分通过测试实际因硬件不可用而提前返回，不能将 11 个通过解释为 11 条硬件链路实测成功。后续主任务发现 DriverStore 中的 64 位驱动 DLL 可用，在隔离工作区内按官方 DLL 别名复制并仅对测试进程临时前置 PATH 后，NVDEC probe 为 13/14 通过，剩余共享互操作失败。此补充证据说明必须区分默认进程环境、隔离测试环境及共享资源互操作结果；默认环境下的能力声明仍须准确反映加载失败。

这一失败使第 1 项的能力误报成为本机可复现风险：Windows LAN 广播只依据编译平台无条件声明 NVDEC 等能力，而当前 NVDEC baseline 实际失败。第 2 项还会令未出帧或 skipped 的运行显示 `zero_copy_enabled=true`。因此，在记录吞吐收益前应先使能力声明与运行结果可信。

本报告区分：

- **避免 CPU 原始帧往返**：GPU 采集、GPU 编码、GPU 解码与共享纹理渲染。GPU 内部格式转换或拷贝仍可能发生。
- **减少压缩 AU 复制**：编码码流在传输、兼容转换、队列和分发处的 CPU 分配与复制。
- **端到端实测证据**：实际运行帧内存域、回退、拷贝计数及吞吐/延迟；不能用配置开关或硬件品牌代替。

以下 P1 表示首先修复的能力/结果可信度问题；P2 表示可落实的主线性能优化。排序先保证结果可信，再删除明显冗余工作。

## 1. [P1] Windows LAN 广播无条件声明硬件能力

**证据**

- `apps/mrd-service/src/lan_discovery/media_capabilities.rs:67`：Windows 分支进入后，在 69–88 行无条件加入 NVENC H.264/HEVC/Main10/AV1、NVDEC H.264/HEVC/Main10/AV1、共享 NV12 和原生渲染等能力。
- `apps/mrd-service/src/lan_discovery.rs:4283`：公告直接使用 `lan_media_capabilities_with_input_control(...)`；1412 行的会话发起能力同样来自该静态入口。
- `apps/mrd-service/src/lan_discovery/media_profile.rs:242`：所需编码能力检查依赖对端声明；Main10 和 AV1 使用相应能力字符串进行接受/拒绝。
- `apps/mrd-service/src/capabilities.rs:1212`、1342：service capability snapshot 已实现真实 NVENC/NVDEC probe，但 LAN 广播没有复用结果。
- 同一 `media_capabilities.rs:123`、153 行已有 macOS 的 probe→能力映射模式，可供结构参考。

**影响**

不支持 AV1/Main10 或 NVDEC 初始化失败的 Windows 机器仍能通过能力协商；失败推迟到会话初始化或解码。当前本机 `nvcuda.dll` 加载失败是具体反例风险。软件 H.264 回退的可用性也不应被硬件能力字符串替代。

**最小方案**

增加 Windows runtime probe 结果结构和纯映射函数，只广播实测通过的编解码/互操作能力，同时广播真实可用的软件回退能力。复用受控缓存，避免每次发现广播都阻塞探测。区分未探测与失败，不能把未探测结果当成可用；设备/驱动变更后应有刷新策略。

**验证**

通过注入 probe 结果覆盖无硬件、仅 H.264、HEVC 无 Main10、AV1 可解不可编、共享纹理失败等矩阵；同时断言公告字符串与 profile 接受/拒绝一致。现有协商测试位于 `apps/mrd-service/src/lan_discovery/tests.rs:4035`、4059、4083、4152。硬件验证恢复后，比较 runtime snapshot 与 LAN 公告，要求当前失败链路不再宣称可用。

## 2. [P1] 零拷贝 benchmark 字段由请求配置推断

**证据**

- `apps/Rdesk/src-tauri/src/benchmark.rs:1870`：`benchmark_zero_copy_enabled` 只检查 decoder、renderer、capture 字符串；没有检查实际帧、编码器输入或 CPU 回退。
- 同文件 1306 行的运行摘要和 1800 行的 skipped 摘要均用该函数填充 `zero_copy_enabled`。
- `apps/Rdesk/src-tauri/src/test_harness.rs:3845`：pipeline comparison 的 `memory_path` 直接从 `config.zero_copy` 得到 `d3d11-shared` 或 `cpu`。
- `test_harness.rs:440` 已有 NVDEC shared-copy 计数，但该计数没有覆盖采集/编码/显示完整路径，也没有用于上述字段。

**影响**

选择 NVDEC+D3D11+DXGI 后，即使根本未出帧或 skipped，结果也可能显示零拷贝已启用；混合路径与 CPU 回退无法从这一布尔值辨认。本机 NVDEC baseline 失败时尤其容易误导验收。

**最小方案**

拆分 requested 与 observed。按实际帧累计 capture、encode input、decode output、render input 的内存域；无运行证据时 observed 为 `null`，发生 CPU 回退或混合路径时不得报告完整链路成功，并记录回退原因。保留 shared-copy 计数作为阶段证据，不将其包装成零 GPU 拷贝结论。

**验证**

增加 skipped、无帧、全 shared、CPU fallback、混合路径测试；确认 JSON/CSV/Markdown 输出一致。现有配置测试位于 `benchmark.rs:2527`，统计导出脚本回归位于 `tests/benchmarks/scripts/test_transport_matrix_common.ps1`。

## 3. [P2] 接收端把已完整 AU 重新分片并本地重组

**证据**

- `apps/mrd-service/src/lan_discovery.rs:6602`：创建 `mux_legacy_fragments`。
- 同文件 6623–6657 行：mux 收到完整视频 envelope，转换为 AU，再于 6638 行编码旧 envelope、6646 行调用 `fragment_access_unit`，将碎片塞入本地队列。
- 同一循环随后重新执行 legacy reassembly，6868 行再次解码 envelope。
- v3 分支在 `lan_discovery.rs:6846` 调用兼容转换；`apps/mrd-service/src/lan_discovery/media_receiver_runtime.rs:93` 先 `frame.payload.to_vec()`，`media_envelope.rs:47` 再复制入 wire buffer，`media_envelope.rs:100` 解 envelope 时再次 `to_vec()`。

**影响**

mux 已提供完整帧，却重新做无网络意义的分片/重组；v3 兼容转换至少产生三次整 AU 复制。这些操作还增加分配、循环和接收队列压力。

**最小方案**

将 mux、v3、v2 收敛为统一的完整待解码 AU 表示，再进入顺序器、关键帧恢复及已有解码流程。mux/v3 直接保留或移动拥有的 payload；v2 仅在实际 wire 边界解析一次。保留 session、lane、profile、codec 校验和 stale-profile 丢弃统计，不通过移除顺序/授权检查来换取性能。

**验证**

复用 `media_receiver.rs:261` 的 transport boundary 测试，以及 `lan_discovery/tests.rs:6991`、7045、7102 的 v3 H.264、HEVC 和 profile mismatch 测试；补充完整 AU 进入公共路径后底层地址不变、乱序、关键帧恢复、旧 v2 兼容及统计一致性。大 AU 的收益应在后续相同参数的 baseline/优化对照中测量，不能仅凭删掉复制推断 FPS 增幅。

## 4. [P2] QUIC 分片和接收接口丢失拥有的 Bytes

**证据**

- `crates/mrd-transport-quic-quinn/src/lib.rs:805`、1031：每片先 `Bytes::copy_from_slice(chunk)`，紧接 `encode()` 又复制到带头 buffer。
- 同文件 734、915 行的 fragment `encode()` 为每片另分配 buffer 并附加 payload。
- 同文件 771、985 行：fragment decode 从 `&[u8]` 重新复制 payload；Quinn 上游本来已提供拥有的 `Bytes`。
- 同文件 1206 附近：完成重组时再将所有片复制到连续 payload；单片帧也走该路径。

**最小方案**

分片直接从借用 chunk 写出 header+payload，避免中间 `Bytes` 分配。添加接收拥有 `Bytes` 的 decode/push 入口，使用 `Bytes::slice`，同时保留旧借用入口兼容。单片完成帧直接移动 payload；多片重组如解码 API 需要连续内存，保留必要的一次合并。

**验证**

要求 v2/v3 wire bytes 一致，覆盖空 payload、MTU 边界、乱序、重复、超时、metadata mismatch 和 byte limits；拥有入口与单片快速路径断言底层 payload 地址共享。避免仅写复述实现的测试。

## 5. [P2] 未安装 Agent 渲染路由仍提前 clone 每个 AU

**证据**

- `apps/mrd-service/src/lan_discovery.rs:6995`：每帧先对 `envelope.payload` 深拷贝，再调用 Agent dispatch。
- `apps/mrd-service/src/app_state/core.rs:291`：dispatch 接受 `Vec<u8>`；309 行才可能发现 MissingSession 并返回 Unavailable，随后主调用路径继续本地解码。

**最小方案**

在同一有效路由保护下判断再复制，或让 dispatch 接受 payload 所有权并在 Unavailable 时归还所有权。不要以独立的“先查路由、稍后发送”制造撤销竞态；保留已安装 Agent 路由在失败时的权威性与禁止本地回退语义。

**验证**

覆盖无路由返回原 payload、有效路由只移交一次、路由撤销拒绝且不触发本地回退。可使用 payload 地址验证没有复制，不需要硬件依赖。

## 6. [P2] D3D11 设备可创建被当作共享纹理互操作证据

**证据**

- `apps/mrd-service/src/capabilities.rs:910`：`memory.d3d11_shared` 复用 `probe_d3d11_render_status`。
- 同文件 1436 行的 probe 只调用 D3D11 renderer factory。
- `crates/mrd-render-d3d11/src/lib.rs:220`：构造器仅创建默认硬件 D3D11 device/context；没有验证共享资源创建/打开、NVDEC CUDA interop、NVENC 注册或同适配器身份。

**最小方案**

分别记录设备可创建与各共享互操作路径的探测结果。记录 adapter LUID、像素格式、支持的 producer/consumer 组合及失败阶段；真正的端到端可用声明必须来自所选适配器上的互操作探测或实际帧证据。

**验证**

纯映射测试覆盖 D3D11 成功但 NVDEC 失败、shared open 失败及不同适配器；本机后续硬件 smoke test 应至少完成资源共享、解码输出并渲染一帧。只创建 renderer 不能通过该验收。

## 后续低优先级项

- `crates/mrd-hardware/src/encoder.rs:174`、203 与 `decoder.rs:155`、184：先 `is_available()` 再 probe，硬件探测重复；耗尽候选或禁用 fallback 时仍返回某个 backend 而非不可用结果。只读搜索未发现产品主线调用这两个 selector，因此不应优先于 service 主线修复。
- `crates/mrd-transport-webrtc/src/lib.rs:1015`、1097、1179、1290、1348：发送端每个 AU 从 `EncodedAccessUnit.bytes: Vec<u8>` 深拷贝成 `Bytes`。后续可增加拥有 `Bytes` 的发送 API 或共享码流表示；不要为这个局部优化立即全仓迁移公共协议。
- `crates/mrd-transport-quic-quinn/src/low_latency.rs:659`、679：已为 `Bytes` 的恢复数据经 `to_vec().into()` 再复制；668 行可用收到 datagram 的 slice。该 FEC 路径需要先确认主线调用和正确性，优先级低于上述正常媒体链路。

## 建议验证命令

以下为实施后的建议命令，不表示本子任务已经运行或通过。请从仓库根目录执行；Rust 首次构建成本可能较高。

```powershell
# QUIC 序列化、分片、重组、流生命周期与集成回归
cargo test -p mrd-transport-quic-quinn

# 公共帧内存域/媒体协议契约
cargo test -p mrd-pipeline-core

# service LAN 能力、profile、完整 AU 边界、接收和路由回归
cargo test -p mrd-service --lib lan_discovery
cargo test -p mrd-service --lib capabilities
cargo test -p mrd-service --lib agent_render

# benchmark requested/observed 语义及结果导出
cargo test --manifest-path apps/Rdesk/src-tauri/Cargo.toml --lib benchmark
powershell -NoProfile -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1
```

执行过滤测试时应检查输出中的实际测试数，防止过滤字符串没有命中却误认为已验证。新增测试的最终名称以实施为准。硬件性能复测须等待本机 NVDEC 初始化问题定位并修复，固定采集源、分辨率、刷新率、codec、bit depth、码率和运行时长，分别记录原始帧 CPU 回退、shared 输出帧、码流复制与吞吐/延迟。没有完成这些实测前，本报告只主张代码中存在可删除的复制及能力声明偏差，不承诺具体性能提升。

## 批准实施后的验证记录

- `crates/mrd-transport-quic-quinn/src/lib.rs:738`、936：分片序列化直接借用压缩帧切片，移除临时 `Bytes` 深拷贝；v2/v3 wire 不变。
- 同文件 756、960、1154、1430：新增 owned decode/reassembly API；借用 API 保留。单片重组直接移动收到的 `Bytes`，多片仍保留一次必要的连续合并，预算与畸形包校验不变。
- `crates/mrd-transport-quic-quinn/tests/owned_payload.rs`：地址共享、单片所有权、golden wire、乱序/重复、多片重组、预算与畸形包回归。
- `apps/mrd-service/src/lan_discovery/media_capabilities.rs:114`：缓存真实 codec runtime probes；编码、解码与各 codec 方向独立映射，D3D11 device 创建不再触发 shared-NV12 广播。软件 H264 使用 `decode.software`，避免旧通用 alias 被 HEVC 协商误用。
- `cargo test -p mrd-transport-quic-quinn --tests`：91 通过，2 个性能测试按原设置忽略；exit 0。
- `cargo test -p mrd-service --lib media_capabilities::tests -- --nocapture`：5 通过、622 filtered；exit 0。测试覆盖无硬件、软件独立回退、部分 codec、Main10 仅接收与本机输入控制。
- `apps/mrd-service/src/capabilities.rs:932`、970：主能力快照也改为独立的 BGRA 共享资源探测，在 producer device 创建共享纹理，通过 producer 实际 adapter 创建第二个 consumer device，然后 `OpenSharedResource` 并检查描述。静态未探测状态为 `Unknown`；运行时成功为 `Available`、失败为 `Unsupported` 并记录具体阶段。label/detail 明确只验证 BGRA 资源共享，不代表 NVDEC、planar、同步或端到端零拷贝。
- 该能力修改的静态回归先红（旧值 `Supported` 与期望 `Unknown` 不符），修改后生产函数精确抽取为独立 rustc test 并链接实际 Windows 0.62 rlib：2 个纯映射测试通过；显式执行真实 BGRA shared create/open 测试通过，实际 0.19 秒。
- **正式 Cargo 回归已通过**：`cargo test -p mrd-service --lib d3d11_shared_ -- --nocapture`，exit 0，5 passed / 1 ignored / 627 filtered；包括 2 个新增能力测试与 3 个现有 shared-frame 回归。真实 BGRA 设备测试显式标记 ignored，可使用 `cargo test -p mrd-service --lib d3d11_shared_actual_bgra_resource_open -- --ignored --nocapture` 重跑。
- **正式 service 模块的实际硬件测试已通过**：从该次 Cargo 生成的 `target/debug/deps/mrd_service-9d80d4f72c7c8c96.exe` 执行 `--exact capabilities::tests::d3d11_shared_actual_bgra_resource_open --ignored --nocapture`，1 passed / 632 filtered，exit 0，实际 0.24 秒。日志：`artifacts/local-zero-copy/service-d3d11-shared-actual-probe.txt`。这证明本机 BGRA D3D11 跨设备资源共享，而本机 NVDEC/CUDA 注册仍独立失败，两者不应合并为同一能力结论。
- 独立只读复查 typed LAN receiver / mux / v3 和 agent 延后载荷复制：未发现本轮改动引入的必修回归。接收前后授权检查、mux 4 MiB 限制与队列预算、session/lane/codec 校验、v3 profile id、丢序后的等待关键帧保留；agent route 状态与序号先于复制检查。`received_bytes` 使用实际 AU 字节数，停止计算此前人为添加的进程内兼容 envelope。

## 本机 CUDA/D3D11 注册失败的独立复现

运行时 DLL 别名准备后，CUDA 与 NVDEC CPU 输出可工作；共享路径失败必须单独报告。以下诊断只创建很小的资源，没有修改驱动、注册表或系统 DLL。

证据位于 `artifacts/local-zero-copy/`：

- `cuda_d3d11_probe.py` / `cuda-d3d11-probe.json`：独立 ctypes Driver API 探针，未导入产品代码，也没有调用 CUVID。
- `cuda-d3d11-probe-direct.json`：绕过 `nvcuda_loader64.dll` 别名，直接加载同 DriverStore 的 `nvcuda64.dll`，结果相同。
- `cuda-d3d11-probe-context-first.json`：CUDA context 先于 D3D11 device 创建，结果相同。
- `cuda_d3d11_native_driver_probe.cpp` / `cuda-d3d11-probe-native-driver.txt`：使用已安装的官方 CUDA 13.1 / Windows SDK 头文件编译的原生 C++ Driver API 交叉验证，结果相同。
- `cuda_d3d11_native_probe.cpp` / `cuda-d3d11-probe-native.txt`：CUDA Runtime API 对照；本机 `cudart64_13.dll` 的 `cudaD3D11GetDevice` 直接返回 801，尚未进入纹理注册，不把它视作一次成功互操作测试。

已验证的边界：

1. CUDA device 0 为 RTX 5060 Ti；CUDA LUID、选中的 DXGI adapter LUID、所创建 D3D11 device 实际 `GetAdapter()` LUID 都是 78988。其它同名适配器有不同 LUID 且 `cuD3D11GetDevice` 返回 100，因此不能按显卡名称匹配。
2. 自建 context 与 primary context 均正确 current 到 device 0，注册前后 `cuCtxSynchronize` 返回成功。
3. R8、R8G8、BGRA 三种官方列出的格式，分别使用无共享、legacy SHARED、NT handle + keyed mutex；Texture2D 与 QI 后的 Resource 指针组合共 36 个注册调用全部返回 101 (`CUDA_ERROR_INVALID_DEVICE`)。资源创建均成功。
4. `cuD3D11GetDevices` 在无 context 和有 current context 时均返回 999；原生 C++ 六个 format/shared 组合也全部注册 101，current device 仍为 0，随后 synchronize 成功。
5. live device 的 NVIDIA D3D11 UMD 与 CUDA driver 来自同一个 DriverStore 目录；`ID3D11On12Device` QI 为 `E_NOINTERFACE`，没有证据说明当前走 D3D11On12。

这些实验排除了本次复现对 Rust/CUVID struct ABI、解码 callback、错误 adapter 名称匹配、共享标记或 Texture2D/base interface 转换的依赖；并没有证明所有 NVIDIA 驱动不支持此功能，也没有定位当前机器的最终系统根因。产品应保留可见 CPU 回退和严格 shared 失败路径，不能静默把该机器标成 NVDEC shared 可用。

官方依据：[CUDA Driver D3D11 interop](https://docs.nvidia.com/cuda/cuda-driver-api/cuda_driver_api/group__CUDA__D3D11.html) 描述 adapter 映射、注册参数与支持的纹理格式；[Microsoft D3D11 resource misc flags](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_resource_misc_flag) 描述共享标志；[D3D11On12 device](https://learn.microsoft.com/en-us/windows/win32/api/d3d11on12/nf-d3d11on12-d3d11on12createdevice) 描述该可查询接口。

复现命令（工作区内，无系统修改）：

```powershell
python artifacts/local-zero-copy/cuda_d3d11_probe.py
# 原生 C++ 可在 x64 Native Tools 命令行执行；本机 CUDA include 为 I:\include。
cl /nologo /EHsc /std:c++17 /I I:\include artifacts/local-zero-copy/cuda_d3d11_native_driver_probe.cpp /Fe:artifacts/local-zero-copy/cuda_d3d11_native_driver_probe.exe /link d3d11.lib dxgi.lib
artifacts/local-zero-copy/cuda_d3d11_native_driver_probe.exe
```
