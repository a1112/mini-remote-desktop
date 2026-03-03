# D3D12/D3D11 硬件编码器架构文档

## 概述

本文档描述了 D3D12/D3D11 硬件编码器集成架构，实现从 D3D12 捕获资源直接到硬件编码器的零拷贝流水线。

## 架构

```
┌─────────────────────────────────────────────────────────────────┐
│                     硬件编码流水线                               │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  [捕获] D3D12 Hybrid Capture (160 FPS)                          │
│     ↓ d3d11_resource (ID3D11Texture2D*)                         │
│                                                                  │
│  [互操作层]                                                      │
│     ↓ CUDA-D3D11 Interop (cuGraphicsD3D11RegisterResource)     │
│     ↓ 注册 D3D11 资源到 CUDA                                    │
│                                                                  │
│  [编码]                                                         │
│     ├─ NVENC (NVIDIA GPUs) - 动态加载                          │
│     ├─ D3D12 Video Encode API (Windows 11 22H2+)                │
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
void* get_hybrid_d3d11_device(HD3D12HybridCapture handle);
void* get_hybrid_d3d11_context(HD3D12HybridCapture handle);
void* get_hybrid_d3d12_device(HD3D12HybridCapture handle);
void* get_hybrid_d3d12_queue(HD3D12HybridCapture handle);
void free_hybrid_capture(HD3D12HybridCapture handle);
```

**Python 封装**: `src/capture/d3d12_hybrid_capture.py`

### 2. NVENC 编码器 (`nvenc_d3d12_dynamic.dll`) - 动态加载版本

**功能**: D3D11 资源直接编码 (通过 CUDA 互操作)

**特点**:
- ✅ 无需 NVENC SDK 编译时依赖
- ✅ 运行时动态加载 nvEncodeAPI64.dll
- ✅ 使用 CUDA-D3D11 互操作 (CUDA 11.0+)
- ✅ 支持运行时回退到软件编码

**导出接口**:
```c
int is_nvenc_supported();
int is_cuda_d3d11_interop_supported();
HNVENCEncoder init_nvenc_encoder_d3d11(
    void* d3d11_device,
    void* d3d11_context,
    const NVENCEncodeConfig* config
);
int encode_nvenc_frame_d3d11(
    HNVENCEncoder handle,
    void* d3d11_texture,
    long long timestamp,
    int force_keyframe
);
int encode_nvenc_frame_cpu(
    HNVENCEncoder handle,
    const unsigned char* data,
    int size,
    long long timestamp,
    int force_keyframe
);
int get_nvenc_encoded_frame(HNVENCEncoder handle, NVENCEncodedFrame* frame);
void free_nvenc_encoder(HNVENCEncoder handle);
```

**依赖**:
- CUDA Toolkit 11.0+ (已安装 v13.0)
- NVIDIA GPU (GTX 1660+ 或 newer)
- nvEncodeAPI64.dll (随 NVIDIA 驱动安装)

**编译**:
```bash
cd cpp_capture
python compile_nvenc_dynamic.py
```

### 3. D3D12 Video Encode API (`d3d12_video_encoder.dll`)

**功能**: Windows 11 原生 D3D12 编码

**系统要求**: Windows 11 22H2+

## 动态加载方案

### 为什么使用动态加载？

1. **无需 SDK**: NVENC SDK 头文件不易获取，驱动自带 nvEncodeAPI64.dll
2. **简化编译**: 不需要配置 NVENC SDK 路径
3. **运行时检测**: 可以优雅回退到软件编码

### 当前实现状态

| 功能 | 状态 | 说明 |
|------|------|------|
| DLL 编译 | ✅ 完成 | 70KB, 使用 CUDA 13.0 |
| 函数导出 | ✅ 完成 | 10/10 函数 |
| CUDA 初始化 | ✅ 完成 | cuCtxCreate (v4 API) |
| D3D11 互操作 | ✅ 完成 | cudaD3D11.h |
| NVENC 检测 | ✅ 完成 | LoadLibrary nvEncodeAPI64.dll |
| 实际编码 | ⚠️  存根 | 返回模拟数据 |

### 下一步 (完整 NVENC 实现)

需要动态加载以下 NVENC API 函数：

```cpp
// NVENC API 函数指针
typedef NVENCSTATUS (NVENCAPI* NvEncodeAPICreateInstanceFunc)(
    NV_ENCODE_API_FUNCTION_LIST*
);

typedef NVENCSTATUS (NVENCAPI* NvEncOpenEncodeSessionExFunc)(
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS*,
    void**
);

// 更多函数...
```

然后：
1. 加载 nvEncodeAPI64.dll
2. 获取函数指针
3. 初始化 NVENC 会话
4. 注册 CUDA 资源
5. 编码帧

## 性能目标

| 指标 | 目标 | 当前状态 |
|------|------|---------|
| 捕获 FPS | 150+ | 160 ✅ |
| 编码 FPS | 120+ | 47 (h264_mf 回退) |
| 端到端 FPS | 100+ | 47 |
| 捕获延迟 | <1ms | 0.09ms ✅ |
| 编码延迟 | <10ms | 21ms |
| CPU 使用 | <30% | ~50% |

## 编译指南

### 前置要求

- Visual Studio 2022
- CUDA Toolkit 11.0+ (已安装 v13.0)
- NVIDIA GPU 驱动 (自带 nvEncodeAPI64.dll)

### 编译命令

```bash
# 设置环境
set CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0

# 编译动态加载版本
python cpp_capture/compile_nvenc_dynamic.py
```

### 编译输出

```
======================================================================
NVENC 动态加载版本编译脚本
======================================================================

[1/4] 检查环境...
  ✅ CUDA Include: C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\include
  ✅ CUDA Lib: C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\lib\x64
  ✅ Visual Studio: D:\Program Files\Microsoft Visual Studio\2022\Community
  ✅ 源文件: nvenc_d3d12_dynamic.cpp
  ✅ nvEncodeAPI64.dll: 存在

[2/4] 编译 nvenc_d3d12_dynamic.dll...
  ✅ 编译成功: nvenc_d3d12_dynamic.dll (70144 bytes)

[4/4] 完成...
✅ nvenc_d3d12_dynamic.dll 已就绪

导出函数测试:
  NVENC 支持: ✅
  CUDA-D3D11 互操作: ✅
  编码器状态: ✅ 就绪 (动态加载模式)
```

## 测试

```bash
# 简单测试
python test_nvenc_simple.py

# 完整流水线测试 (需要捕获 DLL)
python test_nvenc_pipeline.py
```

## 故障排除

### 问题: CUDA 13.0 API 变化

**错误**: `cuCtxCreate_v4`: 函数不接受 3 个参数

**原因**: CUDA 13.0 改变了 cuCtxCreate API

**解决**: 使用新 API
```cpp
// 旧版本 (CUDA < 13.0)
cuCtxCreate(&ctx, 0, device);

// 新版本 (CUDA 13.0+)
cuCtxCreate(&ctx, nullptr, 0, device);
```

### 问题: 找不到 nvcuvid.h

**错误**: `fatal error C1083: 无法打开包括文件: "nvcuvid.h"`

**原因**: NVENC SDK 头文件未安装

**解决**: 使用动态加载方案，无需 SDK 头文件

### 问题: NVENC 初始化失败

**错误**: `nvEncOpenEncodeSession failed: 6`

**原因**:
- GPU 不支持 NVENC
- 驱动版本过低

**解决**:
- 更新 NVIDIA 驱动
- 检查 GPU 兼容性

## 参考资源

- [CUDA-D3D11 互操作](https://docs.nvidia.com/cuda/cuda-c-programming-guide/index.html#interoperability)
- [NVENC API 参考](https://docs.nvidia.com/video-encode/nvenc-api/)
- [D3D12 Video Encode](https://docs.microsoft.com/en-us/windows/win32/direct3d12/d3d12-video-encode)

## 文件清单

```
cpp_capture/
├── nvenc_d3d12_dynamic.h       # 动态加载编码器头文件
├── nvenc_d3d12_dynamic.cpp     # 动态加载编码器实现
├── compile_nvenc_dynamic.py    # 编译脚本
├── d3d12_hybrid_capture.h      # 混合捕获头文件
├── d3d12_hybrid_capture.cpp    # 混合捕获实现
└── nvenc_d3d12_encoder.dll     # 编译输出 (70KB)

test_nvenc_simple.py            # 简单测试脚本
test_nvenc_pipeline.py          # 完整流水线测试
```
