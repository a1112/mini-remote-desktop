# 本机零拷贝优化实施与验证

日期：2026-09-27。范围：当前 Windows 主线的采集、编解码、LAN/QUIC 接收、D3D11 渲染及基准统计。以下结果只适用于已列出的路径；不以本次工作覆盖尚未运行的跨机器、WAN、混合显卡或设备丢失测试。

## 本机结论

硬件为 Intel i9-14900K、约 64 GiB 内存、NVIDIA RTX 5060 Ti 16 GB；Windows 11 专业工作站版 Insider Preview 26300，NVIDIA 驱动 610.88，桌面 2560×1440@144 Hz。CPU 像素回读和 GPU 内部复制分别记录。“零拷贝”统计字段的范围固定为 `cpu_pixel_transfers`，不表示消除了 GPU CopyResource、颜色转换或压缩码流复制。

采集到 H.264 / AV1 NVENC 的共享 BGRA 路径已经本机严格实测通过。三条 1280×720 本机 H.264 管线完成实际解码与呈现，共呈现 361 帧；请求共享的管线正确回退为 CPU 解码，并记录为 mixed / zero_copy_enabled=false。CUDA/NVDEC 的运行库加载路径已经找出并提供进程级修复脚本；但 CUDA 到 D3D11 的纹理注册仍失败，当前不能宣布本机全链路 GPU 零拷贝已打通。

## 已落地的修改

| 环节 | 修改与作用 |
| --- | --- |
| DXGI / WinRT | DXGI 默认提交共享资源更新；WinRT 复用兼容 staging texture，尺寸或格式变化时重建。 |
| NVENC | 首帧及复用槽均先 unmap 再更新纹理，随后 map 提交；输出完成并显式 unlock 后再解除输入映射，失败时保留在途资源。H.264/HEVC 等待私有槽对共享源的 GPU 读取完成；AV1 同步直接注册共享源，完整输出后解映射。CPU BGRA 借用原数据。 |
| NVDEC | 严格共享模式首次出帧前初始化资源；互操作失败显式报错，不静默回读；增加回读/共享复制计数；CUDA 与 D3D11 adapter 匹配；4 槽纹理池和帧租约，未消费槽不复用；CUDA 清理失败的槽隔离并保留所有权。 |
| D3D11 | CPU NV12/P010 使用缓存平面上传和 GPU shader；共享帧租约保留到 GPU query 完成；停止渲染后也能独立回收；按共享资源选择可打开它的本机适配器。 |
| 服务 / Agent 回退 | 软件编码器遇到共享捕获帧时切换 CPU 捕获并重新采帧；H.264/HEVC/AV1 支持关键帧运行时解码器回退；Session Agent 首关键帧也能从共享 NVDEC 回退；接受关键帧但暂未出图的解码器保留状态，允许后续 AU 推进。 |
| QUIC / LAN | 分片去除中间 payload 复制；owned Bytes 接收与单片直接复用；mux 完整 AU 不再本机分片后重组；v3 不再序列化旧 envelope 后重新解析。 |
| Agent 分发 | 在路由授权、激活与序号检查通过后才复制压缩 payload；无路由或被拒绝时不分配，已安装 Agent 路由仍保持权威。 |
| 能力 / 指标 | 能力广播依据缓存的运行时探测；requested 与 observed 分开；无帧、跳过、未知渲染路径不得显示零拷贝成功。 |

完整设计、分工和审计分别见同目录的 `2026-09-27-local-zero-copy-design.md`、`2026-09-27-local-zero-copy-implementation.md` 与各分项审计。

## 可复现运行环境

默认进程的 DLL 搜索路径无法加载 `nvcuda.dll` / `nvcuvid.dll`，但当前安装的 DriverStore 内有对应 64 位驱动。运行以下命令准备工作区内的 DLL 别名，然后在同一个 PowerShell 中启动测试或服务：

```powershell
& tests/benchmarks/scripts/prepare_local_nvidia_runtime.ps1
cargo test -p mrd-encode-nvenc --lib shared_bgra_h264 -- --ignored --nocapture --test-threads=1
```

脚本读取当前注册的 NVIDIA 驱动目录，生成 SHA256/版本清单，只修改当前进程及其子进程的 PATH。输出位于被忽略的 `artifacts/local-zero-copy/nvidia-runtime`。没有安装驱动或修改系统、用户 PATH；从其他终端或现有进程启动的服务不会自动继承它。能力缓存需在运行环境改变后重启相关进程才能刷新。

## 硬件限制的复现

CUDA 设备初始化与 NVDEC CPU 输出能够在隔离运行环境中工作。共享纹理注册 `cuGraphicsD3D11RegisterResource` 返回 101（CUDA_ERROR_INVALID_DEVICE）；`cuD3D11GetDevices` 返回 999。独立探针确认 CUDA 与实际 D3D11 device 的 adapter LUID 一致；custom / primary context、loader / direct driver、不同纹理格式与共享标志都能复现。使用官方头文件的原生 C++ 最小程序也复现，且没有经过本项目 NVDEC 的结构体或回调。

这将问题缩小到本机 CUDA/D3D11 互操作运行环境，但不足以单凭错误码确定是哪一驱动、系统或虚拟显示组件导致。保留严格失败和 CPU 回退，未自动变更显示驱动、重启系统或停用显示设备。原始探针及 JSON 结果保存在 `artifacts/local-zero-copy/`。

## 验证记录

- `mrd-decode-nvdec --test nvdec_probe` 在隔离 NVIDIA runtime 下：**14 通过、1 失败**，3.59 秒。唯一失败是严格共享首帧注册的 101；其 `cpu_readback_frames=0`、`cpu_readback_bytes=0`，没有靠静默回读假装共享成功。允许回退的测试确实读回了 98,304 字节并通过亮度梯度校验，不能算零拷贝。
- `mrd-service --lib`：**628 通过、0 失败、5 ignored**，6.53 秒；原始日志 `artifacts/local-zero-copy/service-lib-final.log`。覆盖协议、认证、Agent 路由、CPU 捕获回退策略、缓冲解码、AV1 候选、P010 输入和完整 AU 复用。首轮失败的非法默认测试 profile 与依赖主机 HEVC 声明的协议 fixture 已修正，生产能力仍按真实探测。
- 同一正式 service 测试程序单独执行 `capabilities::tests::d3d11_shared_actual_bgra_resource_open --ignored`：**1 通过**，0.24 秒，实际验证 BGRA 共享资源创建与跨 device 打开。
- DXGI / WinRT / H264-HEVC NVENC / vendor NVENC 的最终 lib 回归：**48 通过、0 失败、7 ignored**。首帧映射补丁后复跑严格 GPU smoke：CPU BGRA 连续 3 帧、标准 shared BGRA 8 帧、max-speed shared BGRA 8 帧，**3 通过、0 失败、0 ignored**，1.26 秒。这是功能验证，不是吞吐或画质对照。
- AV1 最终 lib 回归：**10 通过、0 失败、2 ignored**。两个严格硬件测试在首帧映射补丁后单独复跑：CPU BGRA 3 帧、同步直接共享 BGRA 4 帧，**2 通过、0 失败、0 ignored**，0.64 秒，实际输出非空 AV1 AU。上述五包 lib 合计 **58 通过、9 ignored**。
- HEVC Main / Main10 严格硬件测试在首帧映射补丁后复跑：**2 通过、0 失败、0 ignored**，0.42 秒，1280×720 CPU BGRA 各连续 3 帧；首帧 SPS 分别解析为 8 位 / 10 位。该结果不覆盖 HEVC shared BGRA 或完整 HDR 链路。上述严格编码硬件合计 **7 通过、0 失败、0 ignored**。
- NVDEC / core / render / D3D11 / OpenGL 相关 lib：**84 通过、0 失败、1 ignored**；解码器工厂 `mrd-decode --test nvdec`：**6 通过**。包括租约、GPU query、NV12/P010 平面内容与失败槽隔离回归。NVDEC 清理失败隔离已由独立代理复核。
- QUIC 完整测试：**91 通过、2 ignored**，包含 wire 等价、owned buffer、单片复用和重组限制回归。
- Session Agent `windows_render`：**10 通过**，覆盖共享解码首关键帧失败后回退、双失败保持拒绝、延迟出帧保留实例和实际后端更新。
- Rdesk 正式 app test binary：`zero_copy_` **23 通过**；完整 harness 组 **64 通过、1 ignored**；benchmark 组 **20 通过**。这些筛选有重叠，不相加为独立测试总数。legacy shared lease **1 通过**；PowerShell 汇总脚本测试通过；三份实际 CSV 的 evidence JSON 解析并与 JSON 结果一致。
- 服务缓冲解码、NVENC 完成顺序、CPU 借用、WinRT 缓存、CUDA 槽隔离和统计假阳性均有先失败再修复的回归证据。详细命令与记录见分项审计。
- 最终独立复核发现并关闭首帧映射问题：vendor 注册 API 会立即 map，原先上传后再次 map 实际没有执行；现在所有 CPU 上传和 H.264/HEVC 私有 shared 槽更新前均先解除映射。新增 5 项回归先失败，再修复；独立复核确认正常顺序及失败资源保留。
- 最终 `cargo check -p mrd-service -p mrd-session-agent` 在首帧映射补丁后通过；日志 `artifacts/local-zero-copy/service-agent-check-final.log`。编码最终测试日志与源文件 SHA256 位于 `artifacts/local-zero-copy/2026-09-27/capture-encode-verification.json` 及 `*-final.log`。
- 最终 `cargo check -p app --bin app` 在同一首帧映射补丁后通过，3.26 秒；证据与编码器源文件 SHA256 位于 `artifacts/local-zero-copy/2026-09-27/final-app-check-first-map.json`。修改文件的 `git diff --check` 通过。

## 本机管线实测

使用正式编译的 debug app test executable，在隔离 NVIDIA runtime 下串行执行三条 H.264 harness 路径；请求 1280×720、30 FPS、4 Mbps、首次呈现后采样 3 秒、loopback 传输、D3D11 呈现。各首个 AU 经独立 OpenH264 解码确认实际尺寸为 1280×720。

| 实测路径 | 呈现帧数 | 请求共享 | 观测路径 | 结果 |
| --- | ---: | --- | --- | --- |
| DXGI → NVENC → 软件 H.264 → D3D11 | 120 | 否 | 四阶段均 CPU，零拷贝 false | 通过、未跳过 |
| Synthetic → NVENC → NVDEC CPU → D3D11 | 121 | 否 | 四阶段均 CPU，零拷贝 false | 通过、未跳过 |
| DXGI shared → NVENC → optional NVDEC shared → D3D11 | 120 | 是 | capture 121 / encode 120 共享；decode / render 各 120 CPU；mixed、零拷贝 false | CPU 回退后通过、未跳过 |

共享请求场景观测约 29.38 FPS；编码 p95 0.64 ms、解码 p95 1.18 ms、呈现间隔 p95 34.90 ms。启动/首次呈现等待也会产生帧，因此不能用呈现帧数除以 3 秒计算 FPS。捕获内容、预热和回退开销不同，以上仅为当前功能与统计验证，不能据此宣称优化前后的提速百分比。

optional harness 每帧重试共享注册，120 次均报 CUDA 101；主线 service / Session Agent 的严格候选会在关键帧失败后切换 CPU 解码器。这次没有扩展 optional harness 的重试策略。通用 harness CPU 渲染适配器仍有颜色转换，因此此结果也不是主线 service 平面上传的性能对照。

汇总证据位于 `artifacts/local-zero-copy/2026-09-27/observed-pipeline-results.json`；各运行的 JSON、CSV、Markdown、manifest 位于 `artifacts/benchmarks/2026-09-27/local-zero-copy-observed/`。证据目录记录了正式可执行文件与实际链接依赖的 SHA256。三条管线运行早于最后的 AV1 生命周期、CUDA 清理失败槽及 NVENC 首帧映射补丁；这些后续补丁分别验证并最终编译检查，不冒充已被三条管线覆盖。

## 边界与后续验证

- GPU 内部复制仍是资源生命周期和布局转换的必要步骤；没有删掉这些复制来换取名称上的“零拷贝”。
- 共享捕获帧仍是同步消费契约。H.264/HEVC 保留私有 GPU 编码槽复制并等待源读取完成；AV1 沿用同步直接注册，输出完成后才正常返回。裸共享句柄没有完整的异步生产者租约，驱动异常时保留纹理不能证明外部生产者永不改写内容。跨进程 GPU 帧传递未在本次实现。
- 永久驱动故障时，无法证明 GPU 已完成的 NVENC native owner 会保留到进程退出，D3D11 完成 worker 也可能保留有界队列。正常路径正常回收；重复创建故障实例可积累资源，不能称此故障路径无泄漏。原生 NVENC 阻塞调用也不受 D3D query 的 2 秒超时限制。
- 未对系统电源计划、BIOS、HAGS、驱动全局设置做没有 A/B 依据的改动。
- Windows 已编译并本机验证；macOS/OpenGL 的匹配分支同步更新，但本机不能验证 macOS 运行结果。
- P010 支持不等于完整 HDR 验收；现有 shader 的色彩范围/矩阵及 HDR 元数据处理没有完成专项测试。Rdesk/legacy 的通用 CPU 渲染适配器仍保留原有颜色转换，主线服务与 Session Agent 可以直接使用 D3D11 平面上传。
- WAN/WebRTC 及 FEC 旁路没有完成同等程度的复制优化；没有验证长时间压力、全部 codec/profile/尺寸/刷新率组合、多 GPU 热切换或网卡 DMA。不能据短时 smoke 保证 144/165 FPS。
- 保留工作区已有移动端、网关和其他未提交更改，没有提交或覆盖这些工作。

## 技术依据

- [Microsoft OpenSharedResource](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11device-opensharedresource)：共享资源跨 device 使用与生产者提交要求。
- [Microsoft Flush](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-flush) 和 [GetData](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-getdata)：Flush 提交命令，event query 用于判断 GPU 完成。
- [NVIDIA NVENC Programming Guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/)：输入资源与输出完成生命周期。
- [NVIDIA CUDA D3D11 Interoperability](https://docs.nvidia.com/cuda/archive/12.4.0/cuda-driver-api/group__CUDA__D3D11.html)：设备映射与资源注册接口。
