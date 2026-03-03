# D3D12 硬件编码器架构文档

## 概述

本文档描述了 D3D12 硬件编码器集成架构，实现从 D3D12 捕获资源直接到硬件编码器的零拷贝流水线。

## 架构

```
┌─────────────────────────────────────────────────────────────────┐
│                     D3D12 硬件编码流水线                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  [捕获] D3D12 Hybrid Capture (160 FPS)                           │
│     ↓ d3d12_resource (ID3D12Resource*)                           │
│                                                                  │
│  [互操作层]                                                      │
│     ↓ CUDA-D3D12 Interop / D3D12 Video Encode API              │
│     ↓ 注册 D3D12 资源到编码器                                   │
│                                                                  │
│  [编码]                                                         │
│     ├─ NVENC (NVIDIA GPUs)                                      │
│     ├─ D3D12 Video Encode API (Windows 11 22H2+)                 │
│     ├─ AMF (AMD GPUs)                                           │
│     └─ Media Foundation (通用回退)                              │
│                                                                  │
│  [输出] H.264/H.265/AV1                                         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## 组件

### 1. D3D12 混合捕获 (`d3d12_hybrid_capture.dll`)

**功能**: D3D11 Desktop Duplication → D3D12 共享资源

**导出接口**:
```c
HD3D12HybridCapture init_hybrid_capture(int monitor, int enable_d3d12);
int capture_hybrid_frame(HD3D12HybridCapture handle, D3D12HybridFrame* frame);
void* get_hybrid_d3d12_device(HD3D12HybridCapture handle);
void* get_hybrid_d3d12_queue(HD3D12HybridCapture handle);
void free_hybrid_capture(HD3D12HybridCapture handle);
```

**Python 封装**: `src/capture/d3d12_hybrid_capture.py`

### 2. NVENC 编码器 (`nvenc_d3d12_encoder.dll`)

**功能**: D3D12 资源直接编码 (通过 CUDA 互操作)

**导出接口**:
```c
int is_nvenc_supported();
int is_cuda_d3d12_interop_supported();
HNVENCEncoder init_nvenc_encoder(void* d3d12_device, void* d3d12_queue, const NVENCEncodeConfig* config);
int encode_nvenc_frame_d3d12(HNVENCEncoder handle, void* d3d12_resource, long long timestamp, int force_keyframe);
void free_nvenc_encoder(HNVENCEncoder handle);
```

**依赖**:
- CUDA Toolkit 11.0+
- NVIDIA Video Codec SDK 11.0+
- NVIDIA GPU (GTX 1660+ 或 newer)

### 3. D3D12 Video Encode API (`d3d12_video_encoder.dll`)

**功能**: Windows 11 原生 D3D12 编码

**系统要求**: Windows 11 22H2+

**导出接口**:
```c
HD3D12Encoder init_d3d12_encoder(void* d3d12_device, const D3D12EncodeConfig* config);
int encode_d3d12_frame(HD3D12Encoder handle, void* d3d12_resource, long long timestamp, int force_keyframe);
void free_d3d12_encoder(HD3D12Encoder handle);
```

## 零拷贝流水线实现

### 步骤 1: 初始化捕获

```python
from capture.dxgi_hybrid import D3D12HybridCapture

capture = D3D12HybridCapture(monitor_index=0, enable_d3d12=True)
d3d12_device = capture.get_d3d12_device()
d3d12_queue = capture.get_d3d12_queue()
```

### 步骤 2: 初始化编码器

```python
# NVENC (需要 CUDA)
from encoder.nvenc_encoder import NVENCEncoder

encoder = NVENCEncoder(
    d3d12_device=d3d12_device,
    d3d12_queue=d3d12_queue,
    width=1920,
    height=1080,
    framerate=60,
    bitrate=5_000_000
)
```

### 步骤 3: 编码循环

```python
while True:
    # 捕获 (返回 D3D12 资源指针)
    frame = capture.capture()
    d3d12_resource = frame.d3d12_resource

    # 编码 (直接使用 D3D12 资源)
    encoder.encode(d3d12_resource, timestamp)

    # 获取编码输出
    encoded = encoder.get_encoded_frame()
```

## 性能目标

| 指标 | 目标 | 当前状态 |
|------|------|---------|
| 捕获 FPS | 150+ | 160 ✅ |
| 编码 FPS | 120+ | 47 (h264_mf 回退) |
| 端到端 FPS | 100+ | 47 |
| 捕获延迟 | <1ms | 0.09ms ✅ |
| 编码延迟 | <10ms | 21ms |
| CPU 使用 | <30% | ~50% |

## 安装 CUDA Toolkit

### 方案 1: 完整安装 (推荐)

1. 下载 CUDA Toolkit 11.8 或更新版本
   - https://developer.nvidia.com/cuda-downloads

2. 安装时选择:
   - ✅ CUDA Toolkit
   - ✅ CUDA Runtime
   - ✅ CUDA Development Tools
   - ✅ Visual Studio Integration

3. 验证安装:
   ```bash
   nvcc --version
   nvidia-smi
   ```

### 方案 2: 仅 CUDA Runtime (最小化)

如果只需要运行，不需要编译:

1. 下载 CUDA Distribution (仅运行时)
2. 解压到目录，添加到 PATH

## 编译 NVENC 编码器

### 前置要求

- Visual Studio 2022
- CUDA Toolkit 11.0+
- NVIDIA Video Codec SDK
  - 下载: https://developer.nvidia.com/nvenc-sdk

### 编译命令

```bash
# 设置环境
set CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v11.8
set NVENC_SDK=C:\nvenc_sdk

# 编译
cl.exe /LD /MD /O2 /EHsc /std:c++17 ^
    /I"%CUDA_PATH%\include" ^
    /I"%NVENC_SDK%\Include" ^
    nvenc_d3d12_encoder.cpp ^
    /link ^
    cuda.lib ^
    nvcuvid.lib ^
    nvEncodeAPI64.lib ^
    /OUT:nvenc_d3d12_encoder.dll
```

## 故障排除

### 问题: NVENC 初始化失败

**错误**: `nvEncOpenEncodeSession failed: 6`

**原因**:
- GPU 不支持 NVENC (GTX 1650+ 或专业卡)
- 驱动版本过低

**解决**:
- 更新 NVIDIA 驱动到最新版本
- 检查 GPU 兼容性

### 问题: CUDA-D3D12 互操作失败

**错误**: `cuGraphicsD3D12RegisterResource failed`

**原因**:
- CUDA 版本过低
- D3D12 资源格式不兼容

**解决**:
- 确保 CUDA 11.2+
- 检查 D3D12 资源格式

## 参考资源

- [CUDA-D3D12 互操作](https://docs.nvidia.com/cuda/cuda-c-programming-guide/index.html#interoperability)
- [NVENC API 参考](https://docs.nvidia.com/video-encode/nvenc-api/)
- [D3D12 Video Encode](https://docs.microsoft.com/en-us/windows/win32/direct3d12/d3d12-video-encode)
