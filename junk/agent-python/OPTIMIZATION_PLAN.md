# GPU Direct 编码性能优化方案

## 当前状态

| 指标 | 数值 | 目标 |
|------|------|------|
| 编码时间 | ~15ms | < 5ms |
| 理论 FPS | 67 | 144 |

## 瓶颈分析

当前流程：
```
WGC 捕获 (~1.3ms)
  → D3D11 Texture
  → CopyResource GPU-GPU (~0.3ms)
  → CUDA Array 注册
  → cuMemcpy2D 到临时缓冲区
  → cuMemcpyDtoH GPU→CPU (~10ms) ← 主瓶颈
  → CPU BGRA→NV12 转换 (~3ms) ← 次瓶颈
  → NVENC 编码 (~2ms)
```

## 三个优化方向

### 1. CUDA Kernel 颜色转换 (GPU 端)

**文件**: `cpp_capture/nvenc_full_cuda_color.cpp`

**实现方案**:
- 使用 CUDA Runtime API 在 GPU 上执行 BGRA→NV12 转换
- 避免数据往返 CPU
- 预期性能提升: 10ms → 1-2ms

**步骤**:
1. 编译 CUDA kernel 为 PTX
2. 使用 cuModuleLoad 加载 PTX
3. 通过 cuLaunchKernel 执行转换

**优点**:
- 完全在 GPU 上执行
- 可并行处理
- NVIDIA GPU 优化

**缺点**:
- 需要编译 CUDA kernel
- 代码复杂度增加

### 2. D3D11 Video Processor (硬件加速)

**文件**: `cpp_capture/d3d11_video_converter.cpp`

**实现方案**:
- 使用 ID3D11VideoProcessor 进行硬件加速颜色转换
- Windows 原生支持，无额外依赖

**步骤**:
1. 创建 D3D11 Video Processor
2. 设置输入视图 (BGRA)
3. 设置输出视图 (NV12)
4. 执行 ProcessVideo

**优点**:
- 硬件加速，零 CPU 占用
- Windows 原生 API
- 可能支持 RGB 输入编码

**缺点**:
- 需要 Windows 8+
- 某些 GPU 可能不支持

### 3. AMF/QuickSync 替代编码器

**文件**:
- `cpp_capture/amf_encoder.h` (AMD)
- `cpp_capture/qsv_encoder.h` (Intel)

**实现方案**:
- AMD AMF: 使用 VCE (Video Coding Engine)
- Intel QuickSync: 使用 Media SDK

**优点**:
- 可能原生支持 RGB 输入
- 无需颜色转换！

**缺点**:
- 需要额外 SDK
- 增加代码复杂度

## 推荐实施顺序

1. **优先级 1**: D3D11 Video Processor (最快实现，零依赖)
2. **优先级 2**: CUDA Kernel (性能最优)
3. **优先级 3**: AMF/QuickSync (增加灵活性)

## 下一步行动

### 立即执行

1. 完善 D3D11 Video Processor 实现
2. 测试性能提升
3. 如效果理想，集成到主编码流程

### 后续规划

1. 实现 CUDA kernel 版本
2. 测试并对比性能
3. 选择最优方案

## 预期结果

使用 D3D11 Video Processor 后：
- 消除 CPU 复制瓶颈
- 消除 CPU 转换瓶颈
- 预期总延迟: 1.3 + 0.3 + 0.5 + 0.5 + 2 = ~4.6ms
- 达到 144fps 目标
