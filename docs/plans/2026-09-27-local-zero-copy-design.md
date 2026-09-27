# 本机零拷贝能力审计与优化设计

日期：2026-09-27。状态：用户已确认“按建议方案全面推进”，开始分层实施。

## 目标与验收

优化 Windows 本机捕获、编码、传输、解码、渲染链路，降低实际数据复制和分配开销，保证帧生命周期、错误处理与软件回退正确。分别记录 CPU 回读、CPU 上传、CPU 字节复制、GPU 内部复制；共享纹理不能自动等同于全链路零拷贝。

验收以实际能力探测、针对性回归测试与可复现硬件测试为依据。硬件不可用必须明确标记，不以条件跳过的测试或后端名称推断硬件成功。保留现有未提交修改与运行中的移动网关。

## 本机基线

- WMI：RTX 5060 Ti，驱动 32.0.16.1088；i9-14900K（24 核 / 32 线程）；约 64 GiB 内存。
- Windows 11 专业工作站版 Insider Preview，10.0.26300；物理显示输出 2560×1440@144Hz；另有多个虚拟显示适配器。
- Rust/Cargo 1.89.0；当前 HEAD `1ef0065`，工作区有其他开发中的修改。
- `cargo test -p mrd-decode-nvdec --test nvdec_probe -- --nocapture --test-threads=1`：11 通过、3 失败；运行时无法加载 `nvcuda.dll`，若干通过项实际提前返回，不能证明硬件解码成功。
- 默认 DLL 搜索路径未找到 `nvcuda.dll`/`nvcuvid.dll`。后续核实当前 DriverStore 有 `nvcuda_loader64.dll`、`nvcuda64.dll`、`nvcuvid64.dll`、`nvEncodeAPI64.dll` 与 `nvml.dll`；使用已安装驱动的进程级别名目录后 CUDA/NVML/NVDEC 可以加载。可复现脚本为 `tests/benchmarks/scripts/prepare_local_nvidia_runtime.ps1`，不改系统 PATH 或安装驱动。CUDA 与 D3D11 互操作仍需独立验证，不能把最初的缺 DLL 诊断视为缺少所有 CUDA 驱动文件。

## 方案比较

1. **建议：分层修复与优化。** 先修资源生命周期、诊断与错误回退，再减少已证实的多余拷贝，最后执行硬件矩阵。保留现有架构，按实际运行能力选择硬件/软件路径。
2. 仅补能力报告和基准。风险与代码改动最小，但不能消除已发现的资源顺序错误和复制开销。
3. 立即重构为严格的 GPU 资源租约管线。潜在收益最大，但需跨捕获、编解码、渲染、IPC 改接口及同步机制；当前缺失的 CUDA 运行库会限制验收。不建议作为首次落地步骤。

## 建议实施范围

### 捕获与编码

- 检查并修复 NVENC 完成顺序：成功锁定输出码流后再解除输入映射，并在出错时保持资源状态可清理。
- 保留既有 NVENC 注册资源池与用户正在开发的低延迟参数。
- 复用 WinRT CPU 回退的 staging 纹理，尺寸或格式变化时重建。
- 检查跨 D3D11 device 的提交和同步；任何省略 GPU CopyResource 的改动必须先满足生产者/消费者生命周期契约。

### 解码与渲染

- 修复硬件探测错误的 codec/阶段上下文。
- 解决共享输出首帧和格式变化时的 CPU 回读，显式记录共享失败与回退。
- 修复 CPU NV12 软件/硬件解码回退与 D3D11 渲染器的输入契约不一致；采用可缓存的平面上传，避免增加 CPU RGB 转换。
- 为异步共享帧补齐资源所有权/消费完成保障；未完成这一契约前不得宣布严格 GPU 零拷贝。

### 传输与能力

- 移除 QUIC 分片的中间字节复制，同时保持 wire format 和边界验证。
- 核查主线 mux 接收后在本机重复分片/重组的兼容转换，统一待解码数据入口时保留认证、profile 与顺序检查。
- LAN 能力广播采用真实探测结果，不能因 Windows 平台就宣布 NVIDIA/AV1/Main10 支持。
- 零拷贝指标必须包含实际编码输入与运行时路径；不可验证时使用 unknown/不可用，而非 true。

## 验证策略

1. 保存现状审计，针对每项可复现缺陷先添加失败回归测试。
2. 分模块构建和执行测试；硬件基准串行运行，避免并发干扰。
3. 能运行时验证 DXGI/WinRT、H264/HEVC/AV1、NV12/P010、D3D11、QUIC/WebRTC，以及 CPU 回退。
4. 记录成功帧数、错误/回退次数、CPU 回读字节数、上传次数、分配/拷贝变化、吞吐与 p50/p95 延迟。测试跳过不计为性能达标。
5. 给出本机可复现命令、实际结果、限制与尚需硬件验证的项目。

## 官方依据

- [Microsoft Desktop Duplication API](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api)：桌面帧在 DXGI surface 中提供。
- [Microsoft Surface Sharing](https://learn.microsoft.com/en-us/windows/win32/direct3darticles/surface-sharing-between-windows-graphics-apis)：共享资源需要生产者/消费者同步。
- [NVIDIA NVENC Programming Guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/) 及仓库 SDK `nvEncodeAPI.h`：输入注册、映射、编码完成与解除映射具有生命周期要求。
