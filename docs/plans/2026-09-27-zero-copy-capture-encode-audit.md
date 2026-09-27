# 本机捕获与编码零拷贝审计

审计日期：2026-09-27。范围为 `crates/mrd-capture-dxgi`、`crates/mrd-capture-winrt`、`crates/mrd-encode-nvenc`、`crates/mrd-encode-nvenc-av1`、`crates/mrd-encode-openh264`、`crates/mrd-pipeline-core`，以及主线 service 的输入协商调用点。未以 `junk/` 或历史子项目作为架构依据。

以下问题位置是实施前的行号快照；实施完成后的验证记录见本文末尾。

## 已确认的路径

H264/HEVC 全彩 shared BGRA 捕获到编码没有正常路径的 CPU 像素回读，但仍有两次 GPU 复制：

1. DXGI `crates/mrd-capture-dxgi/src/lib.rs:597` 或 WinRT `crates/mrd-capture-winrt/src/windows_impl.rs:724` 将系统捕获纹理复制到自有共享纹理。
2. NVENC H264 `crates/mrd-encode-nvenc/src/lib.rs:823`、HEVC `:1346` 将共享纹理复制到已注册的编码槽。

第二次复制目前隔离了捕获纹理被覆盖和异步编码输入的生命周期，不能直接删除。`crates/mrd-pipeline-core/src/lib.rs:128` 的共享 BGRA 帧只有裸句柄、尺寸与 pitch，没有资源租约、adapter LUID、generation 或完成信号。DXGI 为三个纹理循环复用，WinRT 仅使用一个共享纹理。这些异步 H264/HEVC 路径的证据支持“无 CPU 回读”，不支持“零 GPU 复制”或直接移除私有编码槽。独立的 AV1 实现同步直接注册捕获共享纹理，没有第二次编码端 GPU 复制；本轮修复和验证见本文最后的 AV1 补充。

已有有效优化：NVENC 缓存打开的共享资源（上限 8）、H264 两个与 HEVC 三个预注册编码槽；OpenH264 复用 I420 scratch。没有发现每帧 NVENC 注册的问题。

## 优先级与最小修改

| 优先级 | 已确认问题 | 最小修改 |
| --- | --- | --- |
| P1 | NVENC H264 `lib.rs:757`、HEVC `:1275`、同步 CPU helper `:1985` 先 unmap，再等待 bitstream 完成。SDK `tools/Video_Codec_Interface_13.0.37/Interface/nvEncodeAPI.h:4164` 要求 lock 成功后才 unmap。 | 统一 lock/copy → unmap → recycle；完成失败时保留 pending slot，防止在途资源被提前释放。 |
| P1 | DXGI `lib.rs:620` 的 producer Flush 默认关闭（`:883`），而 NVENC 在另一 D3D11 device 打开该共享纹理。 | 默认提交 producer 更新；明确的诊断 opt-out 可以保留，但不能把关闭提交的结果作为正确性默认。 |
| P1 | `apps/mrd-service/src/lan_discovery/media_capture_config.rs:109` 无条件将 display-shared 选为 GPU 捕获；NVENC 初始化失败可回退 OpenH264，但 `lan_discovery.rs:5348` 仅替换编码器，`:5410` 仍传入 shared frame 的空 data。 | 按实际编码器输入内存能力重新建立 CPU capture 并重新采帧；不能把 GPU frame 直接送软件编码器。 |
| P2 | WinRT `windows_impl.rs:644` 每帧创建 staging texture。 | 缓存兼容 staging texture；尺寸、格式及其他 CopyResource 兼容属性变化时重建。 |
| P2 | NVENC `lib.rs:2273` 对已有 CPU BGRA 做完整 clone，再上传。 | BGRA 借用原切片，只有需要格式转换时拥有新缓冲。 |
| 后续 | DXGI 使用显示器 adapter（`lib.rs:300`），NVENC 固定默认 adapter（`mrd-encode-nvenc/src/lib.rs:1896`）。 | 引入 adapter 身份和同 GPU 协商；不能假定混合显卡默认设备具备 NVENC 或可打开共享资源。 |
| 后续 | 窗口缩放由 `media_capture_config.rs:129` 强制回 CPU。 | 增加 GPU 缩放和帧资源租约，再逐步消除第二次 GPU copy。 |

每消除一次 BGRA 全帧复制，可减少的理论载荷为 1080p60 0.498 GB/s、4K60 1.991 GB/s、4K120 3.981 GB/s。这是像素字节数乘帧率，不是实测内存带宽或性能提升；实际复制还涉及读写流量。

## 验证范围与限制

- 纯单测可验证默认策略、输入借用、完成顺序及失败队列保留。
- Windows WARP 软件 D3D11 可实际验证 staging 资源复用与 resize/format 失效，不依赖 NVIDIA 硬件。
- NVENC 存在运行库不等于编码成功。需要严格硬件 smoke，不能把提前返回的条件测试记作编码通过。
- 现有 `mrd-encode-nvenc/tests/perf_encode.rs` 只使用 CPU BGRA 输入，无法测出共享纹理路径的收益。
- 捕获 benchmark 若反复返回缓存帧，极小 latency 不能外推为新画面帧率。
- 真正验证跨设备帧内容需要交替图案捕获 → 编码 → 解码内容校验，并覆盖超过环形槽数量、resize、源切换、capture 提前销毁、消费者延迟和设备故障。
- 在资源租约与同步契约落地前，保留 GPU 复制作为生命周期边界，禁止报告严格零 GPU copy。

## 建议命令

```powershell
cargo test -p mrd-encode-nvenc --lib
cargo test -p mrd-capture-dxgi --lib
cargo test -p mrd-capture-winrt --lib
cargo test -p mrd-encode-openh264
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/capture.dxgi_shared.json
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/capture.winrt_monitor_shared.json
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 -ScenarioPath tests/benchmarks/scenarios/quick.transport.webrtc.nvenc.h264_nvdec.json
```

GPU benchmark 应串行执行。DLL 诊断只在测试进程 PATH 加入根代理准备的运行库隔离目录，不修改系统 PATH 或安装驱动。

## 保留的用户修改

实施前已存在 `NvencH264Encoder` max-speed 构造的 `set_h264_zero_reorder_delay()` 调用，以及 `vendor/nvenc/src/sys/structs.rs` 对应实现与回归测试。本任务保留这些修改且不触碰该用户修改文件。异常清理审查后，最小扩展了独立的 `vendor/nvenc/src/safe/bitstream.rs`，增加显式 unlock 错误传播与无 panic 的析构清理。

## 依据

- [Microsoft OpenSharedResource](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11device-opensharedresource)：跨设备共享纹理的 producer 更新需要 Flush。
- [NVIDIA Video Codec SDK 13 programming guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/) 和仓库 SDK 头文件：资源注册、映射与输出完成生命周期。

## 实施验证记录

以下行号对应最终实施代码：

| 修改 | 位置与行为 |
| --- | --- |
| 输出完成顺序 | `crates/mrd-encode-nvenc/src/lib.rs:2618`、`:2679`：lock → copy bitstream → 显式 unlock → input unmap → 回收槽。H264、HEVC 和 CPU 同步 helper 共用。 |
| CPU 输入状态 | `lib.rs:191`、`:258`：`PendingEncodeResource` 在提交前标记在途，仅完整完成后清除。上传前 `:1683`、`:1750` 检查 idle，失败后拒绝覆盖原纹理。 |
| 共享提交失败 | `lib.rs:2571`：先让 pending 队列持有槽，再调用驱动提交。提交返回错误不会析构局部槽。 |
| 共享退出失败 | `lib.rs:2596`：仅成功完成才出队；永久失败保留整个未完成队列中的槽到进程退出，保留 encoder Arc 和 backing texture。 |
| CPU 退出失败 | `lib.rs:956`、`:1493`：同步调用只会在错误后仍 pending。Drop 不重复阻塞 lock；状态 guard 保留整槽并输出明确诊断。 |
| 显式解锁与清理 | `vendor/nvenc/src/safe/bitstream.rs:50` 显式 unlock 返回错误且只尝试一次；`:87` 清理错误写 stderr，避免 native error 引发二次 panic。原有用户 `sys/structs.rs` 修改保留。 |
| 来源纹理读取完成 | `lib.rs:272` 的 `SharedInputCopyCompletion` 缓存 D3D11 event query。共享源复制/着色后等待 GPU 读完成才允许正常返回，避免 WinRT 单纹理被下一帧覆盖；等待失败/超过 2 秒明确报错。NVENC 编码仍可异步使用独立编码槽。 |
| producer 提交 | `crates/mrd-capture-dxgi/src/lib.rs:883` 默认 Flush，仅明确 `0/false/no/off` 诊断设置关闭。 |
| staging 复用 | `crates/mrd-capture-winrt/src/windows_impl.rs:119` 缓存规范化完整描述与纹理；`:676` 复用；停采或切到 shared 清除缓存。 |
| CPU BGRA 借用 | `lib.rs:2909` 的 `to_bgra` 返回 `Cow<[u8]>`，BGRA 借用原切片，必要格式转换仍拥有新缓冲。 |

NVENC 析构先提交 EOS，再完成 shared 尾帧。资源隔离是一项极端故障策略：无法证明完成的槽故意不执行 native unmap/unregister/destroy，保留到进程退出；正常完成路径正常回收。连续创建多个故障编码器会积累被保留的资源，不能称此错误路径无泄漏。

回归红灯证据：DXGI 默认提交测试在原实现下失败；NVENC 的完成顺序、lock 失败保留映射、完成失败保留队列、unmap 失败保留槽、BGRA 借用共 5 个测试失败；WinRT 同描述复用测试观察到不同原生纹理地址而失败。共享退出失败测试先观察到全部 3 个槽析构，修复后仅析构已完成的 1 个。共享源等待的纯状态测试和 WARP event query 测试均先捕捉过 GPU 未完成即放行。

最后一轮异常回归先实测 **CPU 3 通过、2 失败、1 忽略**（两个错误路径均提前析构，计数 1 而非 0）、**shared submission 1 失败**（失败后队列为空）、**vendor cleanup 2 失败**（unlock 尝试两次及 cleanup panic）。然后修复并执行统一绿测：

```powershell
cargo test -p nvenc -p mrd-encode-nvenc -p mrd-capture-winrt -p mrd-capture-dxgi --lib --no-fail-fast
```

| 包 | 通过 | 失败 | 忽略 |
| --- | ---: | ---: | ---: |
| mrd-capture-dxgi | 8 | 0 | 0 |
| mrd-capture-winrt | 12 | 0 | 4 |
| mrd-encode-nvenc | 19 | 0 | 3 |
| nvenc | 6 | 0 | 0 |
| 合计 | **45** | **0** | **7** |

WinRT 两项资源测试与 NVENC copy-completion 测试实际使用 WARP 软件 D3D11，未跳过。4 个 WinRT 忽略项为交互/捕获探针；3 个 NVENC 忽略项在下面的严格硬件测试中单独执行。最终 scoped rustfmt 与 owned files 的 `git diff --check` 通过（只有 LF→CRLF 提示）。

真实硬件复现命令（仅当前进程 PATH，不修改系统）：

```powershell
$nvencRuntimePath = Join-Path (Get-Location) 'artifacts/local-zero-copy/2026-09-27/runtime'
$env:PATH = $nvencRuntimePath + ';' + $env:PATH
cargo test -p mrd-encode-nvenc --lib bgra_h264 -- --ignored --nocapture --test-threads=1
```

首帧映射补丁前实际运行的是该轮绿测刚构建的 `target/debug/deps/mrd_encode_nvenc-2d0eba8d94f762d0.exe`，同一 `bgra_h264 --ignored --nocapture --test-threads=1` 参数，避免再争用 Cargo 锁。**3 通过、0 失败、0 忽略，测试执行 0.68 秒**：

- 标准 H264 CPU BGRA：1280×720、连续 3 帧，各产生 1 个非空访问单元并验证时间戳、首帧关键帧与 CPU 槽恢复 idle。
- 标准 H264 shared BGRA：1280×720、8 帧，独立 producer D3D11 device；输出 7 个非空访问单元、时间戳顺序和首帧关键帧正确，析构完成尾帧。
- max-speed H264 shared BGRA：同样 8 帧与上述断言。

这是功能 smoke，不是吞吐基准，没有解码像素内容对照。较早的 128×128 max-speed 探针初始化返回 `InvalidParam`，没有编码帧；改为现有正式 probe 的 1280×720 后两条构造均通过。不能把小尺寸失败泛化为硬件不可用，也不能声称所有尺寸均支持。

仍未覆盖：向真实驱动注入永久故障后的 teardown、设备丢失、混合显卡/多 adapter、HEVC/Main10 真实共享纹理、实际捕获图案编码解码内容一致性。HEVC Main/Main10 的 CPU BGRA 实际编码与 SPS 位深已在本文最后的严格补测中通过。原生 NVENC `try_lock(true)` 的阻塞时长不受 D3D query 的 2 秒超时保护；本轮保证的是已返回错误时的状态/所有权与非 panic 清理，不是驱动所有调用的可中断性。

主线 service 的软件编码回退和通用解码器回退由根代理修改。本代理只读交叉审查确认 CPU capture 重建后重新采帧，避免共享空 data 进入软件编码器；另指出首次关键帧被 decoder 接受但暂不输出时不应丢弃实例，根代理已修改为保留 decoder，并在主循环清除关键帧等待 gate。其运行验证数字由根代理报告。

## AV1 独立实现补充

扩展审查发现 AV1 是独立实现，H264/HEVC 的修复不会自动覆盖它。原 AV1 路径长期保持输入 mapped，输出仅依赖 lock guard Drop 解锁，encoder Drop 先 unmap 再 EOS；CPU BGRA 全帧 clone，且缺少长度校验。

已修改 `crates/mrd-encode-nvenc-av1/src/lib.rs`：

- `:42`、`:71`：全部 native owner（bitstream、CPU/shared 注册、encoder、纹理、context、device）置于 `RetainedOnFailure<NativeResources>`。提交前 pending，完整输出完成后才清除；任何提交/lock/unlock/unmap 错误或 unwind 保持 pending，禁止下一次 encode 打开新 shared input 或覆盖 CPU texture。
- `:110`：错误退出时保留整个 owner 到进程退出，并输出诊断；不再次调用已失效驱动。`:858` 的正常 owner Drop 在 EOS 后处理未使用的初始映射，字段再按 bitstream/注册/纹理顺序释放。
- `:994`：每帧 map → submit → lock/copy → 显式 unlock → unmap，显式错误经 `:124` completion helper 传播。同步成功返回表示当前共享源编码已完成，不增加 GPU copy。
- `:1044`：BGRA 通过 `Cow` 借用，并验证输入字节数；RGBA/RGB/NV12 执行必要转换。

AV1 的 direct shared 路径和 H264/HEVC 私有槽路径不同：捕获端仍需系统纹理到自有共享纹理的 GPU copy，AV1 编码端直接注册该共享纹理，同步等待完成。错误后不再复用该 encoder；保留 opened texture 能防止提前释放，但裸 shared handle 不提供外部 producer 的覆盖租约，真实驱动故障后外部再次写源纹理的内容一致性未验证。本轮按批准范围没有扩展 producer lease 架构。

RED：`cargo test -p mrd-encode-nvenc-av1 --lib --no-fail-fast` 实际 **3 通过、5 失败、2 忽略**，分别捕捉完成缺少 unmap、unmap 错误被吞、native owner 提前析构、BGRA clone 和截断输入被接受。修复后同命令 **8 通过、0 失败、2 忽略**。该轮（首帧映射补丁前）捕获/编码/vendor 普通定向验证合计为 **53 通过、0 失败、9 忽略**（45 项前序统一测试 + AV1 8 项）。

首帧映射补丁前的两个严格探针通过该轮新构建的 `target/debug/deps/mrd_encode_nvenc_av1-3c72e279f65dd484.exe` 直接运行，参数 `strict_av1 --ignored --nocapture --test-threads=1`，同一进程 PATH 隔离目录。1280×720 CPU BGRA 连续 3 帧、独立 producer device 的 shared BGRA 连续 4 帧，均每帧得到非空 AV1 访问单元并验证时间戳，**2 通过、0 失败、0 忽略，执行 1.07 秒**。可重现命令：

```powershell
$env:PATH = (Join-Path (Get-Location) 'artifacts/local-zero-copy/2026-09-27/runtime') + ';' + $env:PATH
cargo test -p mrd-encode-nvenc-av1 --lib strict_av1 -- --ignored --nocapture --test-threads=1
```

本机 AV1 的实际正常构造与编码成功，现有 GUID/preset capability probe 在该配置上未观察到虚报。此结果不替代其他设备/driver 上的实际编码验证，也没有验证 AV1 解码像素内容。所有 AV1 源码均已 scoped rustfmt，diff whitespace 检查通过。

## HEVC / Main10 严格硬件补测

`crates/mrd-encode-nvenc/tests/nvenc_encoder.rs:317`、`:326` 新增两个 ignored strict wrapper，不调用旧 `when_available` 测试，构造必须 `expect` 成功。两条路径均使用 1280×720 CPU BGRA，连续编码 3 帧，校验每帧非空 HEVC 访问单元与时间戳，并解析第一帧 SPS：Main 必须 8 位，Main10 必须 10 位。旧条件测试（包括提前 return 的 Main10 测试）没有计入硬件成功。

```powershell
$env:PATH = (Join-Path (Get-Location) 'artifacts/local-zero-copy/2026-09-27/runtime') + ';' + $env:PATH
cargo test -p mrd-encode-nvenc --test nvenc_encoder strict_hevc -- --ignored --nocapture --test-threads=1
```

首帧映射补丁前结果 **2 通过、0 失败、0 忽略，执行 0.49 秒**。完整本轮命令输出保存于 `artifacts/local-zero-copy/2026-09-27/nvenc-hevc-strict.log`。HEVC/Main10 CPU 编码能力已真实验证，共享输入模式尚未单独做 HEVC 内容校验。

首帧映射补丁前本分工验证汇总：普通捕获/编码/vendor **53 通过、0 失败、9 忽略**；严格实际硬件编码 **7 通过、0 失败、0 忽略**（H264 CPU 1 + H264 shared 2 + AV1 CPU/shared 2 + HEVC Main/Main10 CPU 2）。硬件输出可用不等于端到端严格零拷贝；H264/HEVC 保留私有槽复制，AV1 同步直接注册，码流输出始终会从 NVENC 锁定缓冲复制到 Rust Vec。未提交 Git，也未改系统 PATH/驱动。

## 最终首帧映射同步修复与复测

最终独立审查确认，vendor 的 `register_resource_dx11` 会立即映射资源。因此仅有每帧 encode 前 map 还不够：首帧 CPU upload、新建 H264/HEVC 私有槽的第一次 CopyResource/Draw，都必须先释放初始映射。SDK `nvEncodeAPI.h:4126` 的 map 同时承担等待此前 graphics work 完成的同步边界。本轮不修改 vendor 的全局注册 API。

- H264/HEVC：`crates/mrd-encode-nvenc/src/lib.rs:230` 的 `prepare_for_upload` 先在 guard 中标记 pending，再 unmap，成功才恢复 idle；失败保留原生槽。CPU 两入口 `:1683`、`:1750` 在 UpdateSubresource 前统一调用，不依赖哪个构造器。私有 shared 槽 `:841`、`:1377` 在任何 CopyResource/Draw 前通过 `:241` 的 guard 移交完成同样准备。回收槽已经 unmapped 时仅做 no-op；写入之后 submit 才重新 map。
- AV1：`crates/mrd-encode-nvenc-av1/src/lib.rs:133` 的 `prepare_cpu_input` 在 `RetainedOnFailure::run` 内先 unmap，再 upload，再进入 `:994` 的 map/submit/输出完成链。初始 unmap 失败不会上传，也不会释放 native owner。shared 源初次注册发生在 producer 更新后，保持直接注册路径。

新增 5 个回归先实际全部失败：NVENC CPU 首次顺序、CPU 初始 unmap 失败保留、shared 私有槽初始 unmap 失败保留；AV1 首次顺序及初始 unmap 失败禁止上传。修复后最终统一命令：

```powershell
cargo test -p nvenc -p mrd-encode-nvenc -p mrd-encode-nvenc-av1 -p mrd-capture-winrt -p mrd-capture-dxgi --lib --no-fail-fast
```

**最终 58 通过、0 失败、9 忽略**：DXGI 8；WinRT 12 + 4 忽略；NVENC H264/HEVC 22 + 3 忽略；AV1 10 + 2 忽略；vendor nvenc 6。日志 `artifacts/local-zero-copy/2026-09-27/capture-encode-lib-final.log`。

全部 7 个严格真实 GPU 编码探针在最终新源码上复跑：H264 CPU/shared/max-speed shared **3 通过，1.26 秒**；AV1 CPU/direct shared **2 通过，0.64 秒**；重编 HEVC integration 后 Main/Main10 CPU **2 通过，0.42 秒**，首帧 SPS 分别 8/10 位。**0 失败、0 忽略**。原先管线 benchmark 是首帧补丁前的结果，没有作为本补丁新的性能收益证据。

原始输出分别保存在 `nvenc-h264-strict-final.log`、`nvenc-av1-strict-final.log`、`nvenc-hevc-strict-final.log`，均位于 `artifacts/local-zero-copy/2026-09-27/`。该目录 `capture-encode-verification.json` 包含最终计数、命令、test exe 与源码 SHA256。独立只读复查确认最初映射问题已修复；最终 owned files 的 `git diff --check` 通过。
