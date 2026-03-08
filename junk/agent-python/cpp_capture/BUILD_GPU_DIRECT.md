# GPU Direct 编译指南

本文档说明如何编译 C++ DLL 以支持 D3D11 Direct (Zero Copy) 传输。

## 架构概述

```
┌─────────────────────────────────────────────────────────────┐
│                    GPU Direct 管道                          │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐ │
│  │ DXGI Capture │───▶│ D3D11 Texture│───▶│   NVENC      │ │
│  │ (Desktop Dup)│    │ (GPU Memory) │    │  (Hardware)  │ │
│  └──────────────┘    └──────────────┘    └──────────────┘ │
│                                               │             │
│                                               ▼             │
│                                       ┌──────────────┐     │
│                                       │ H.264 Output │     │
│                                       └──────────────┘     │
│                                                             │
│  特点: 零 CPU 拷贝, 完全在 GPU 上处理                       │
└─────────────────────────────────────────────────────────────┘
```

## 依赖要求

### 1. Visual Studio 2019/2022
- **工作负载**: "使用 C++ 的桌面开发"
- **组件**: Windows 10/11 SDK, C++ CMake 工具

### 2. CUDA Toolkit 12.x
- 下载: https://developer.nvidia.com/cuda-downloads
- 默认路径: `C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6`
- 包含: CUDA Runtime, NVENC SDK

### 3. NVENC SDK 13.0
- 包含在 CUDA Toolkit 中
- 或单独下载: [Video Codec SDK](https://developer.nvidia.com/nvidia-video-codec-sdk)
- 项目路径: `J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37`

### 4. CMake 3.15+
- 下载: https://cmake.org/download/
- 或通过 Visual Studio Installer 安装

## 快速开始

### 方法 1: 使用构建脚本 (推荐)

```batch
# 打开 "x64 Native Tools Command Prompt for VS 2022"
cd J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture
build.bat
```

### 方法 2: 手动编译

```batch
# 1. 打开 "x64 Native Tools Command Prompt for VS 2022"

# 2. 进入目录
cd J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture

# 3. 创建构建目录
mkdir build
cd build

# 4. 配置 CMake (调整路径)
cmake .. -G "Visual Studio 17 2022" -A x64 ^
    -DNVENC_SDK_PATH="J:/ProjectTest/远程探查/mini-remote-desktop/tools/Video_Codec_Interface_13.0.37/Interface" ^
    -DCUDA_PATH="C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v12.6"

# 5. 编译
cmake --build . --config Release

# 6. 复制 DLL
copy Release\*.dll ..\
```

## 输出文件

编译成功后会生成以下 DLL 文件:

| DLL 文件 | 用途 |
|---------|------|
| `d3d12_hybrid_capture.dll` | DXGI 捕获 + D3D11 设备获取 |
| `nvenc_full.dll` | NVENC 编码器 (D3D11-CUDA 互操作) |
| `dxgi_capture.dll` | 基础 DXGI 捕获 (备用) |

这些 DLL 会被自动复制到 `agent-python` 目录。

## GPU Direct Python 集成

### 1. 捕获器初始化

```python
from src.capture.hybrid_capture import create_hybrid_capture

# 创建混合捕获器
capture = create_hybrid_capture(monitor_index=0)
capture.initialize()

# 获取 D3D11 设备 (用于 NVENC)
d3d11_device = capture.get_d3d11_device()
d3d11_context = capture.get_d3d11_context()
```

### 2. NVENC 编码器初始化

```python
from src.encoder.nvenc_encoder import create_nvenc_encoder

# 创建 NVENC 编码器 (D3D11 模式)
encoder = create_nvenc_encoder(
    d3d11_device=d3d11_device,
    d3d11_context=d3d11_context,
    width=1920,
    height=1080,
    quality=24,
    framerate=60
)
```

### 3. GPU Direct 编码循环

```python
while True:
    # 捕获帧 (返回 D3D11 纹理)
    frame_info = capture.capture_frame()
    texture_ptr = capture.get_texture_ptr()

    # 直接从 D3D11 纹理编码 (零拷贝!)
    encoded = encoder.encode_d3d11(texture_ptr)

    if encoded:
        # 发送编码后的 H.264 数据
        transport.send(encoded.data)
```

## 性能预期

| 分辨率 | 目标 FPS | 帧时间 | 带宽 |
|-------|---------|--------|------|
| 720p  | 120+    | ~3ms   | ~2 Mbps |
| 1080p | 60+     | ~8ms   | ~5 Mbps |
| 1440p | 45+     | ~15ms  | ~10 Mbps |
| 4K    | 30+     | ~25ms  | ~25 Mbps |

## 故障排除

### DLL 未找到

```
错误: Hybrid capture DLL not found: d3d12_hybrid_capture.dll
解决: 先编译 C++ DLL (cd cpp_capture && build.bat)
```

### CUDA 头文件未找到

```
错误: cuda.h: No such file or directory
解决: 安装 CUDA Toolkit 或调整 CUDA_PATH
```

### NVENC SDK 未找到

```
错误: nvEncodeAPI.h: No such file or directory
解决: 设置 NVENC_SDK_PATH 指向 Video_Codec_SDK/Interface
```

### 编码器初始化失败

```
错误: Failed to initialize NVENC encoder
解决: 检查 NVIDIA 驱动是否支持 NVENC (GTX 1650+)
```

## 技术细节

### D3D11-CUDA 互操作

```cpp
// 注册 D3D11 纹理到 CUDA
cuGraphicsD3D11RegisterResource(&cuda_resource, d3d11_texture, ...);

// 映射到 CUDA 指针
cuGraphicsResourceGetMappedPointer(&cuda_ptr, &size, cuda_resource);

// 传递给 NVENC
nvEncRegisterResource(nvenc_encoder, d3d11_texture);
```

### 零拷贝流程

1. **DXGI 捕获**: 直接写入 GPU 内存
2. **D3D11 纹理**: 保持在 GPU 上
3. **CUDA 映射**: 获取 GPU 内存地址
4. **NVENC 编码**: 直接访问 GPU 内存
5. **H.264 输出**: 编码后复制到 CPU

整个过程中，原始帧数据**从未**离开 GPU。

## 测试

运行 GPU Direct 测试:

```bash
python test_gpu_direct.py
```

预期输出:

```
======================================================================
GPU DIRECT 管道测试结果
======================================================================

持续时间:       3.00 秒
编码帧数:       180
实际 FPS:       60.0
目标 FPS:       60

平均帧时间:     8.50 ms
  - 捕获+编码:   8.50 ms
  - 纯编码:     2.00 ms

平均帧大小:     15,234 字节
带宽:           7.3 Mbps

🚀 卓越 (55+ fps)
状态: GPU Direct 管道工作完美!
```

## 参考资料

- [NVENC SDK](https://developer.nvidia.com/nvidia-video-codec-sdk)
- [DXGI Desktop Duplication API](https://docs.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api)
- [CUDA-D3D11 Interop](https://docs.nvidia.com/cuda/cuda-runtime-api/group__CUDART__INTEROP.html)
