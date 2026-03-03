# 屏幕捕获方案对比与 DXGI 集成总结

## 可用的捕获方案

| 方案 | 速度 | 零拷贝 | Python 3.12 | 状态 |
|------|------|--------|-------------|------|
| DXGI Desktop Duplication | 120+ FPS | ✅ | ❌ 需要编译 | 不可用 |
| d3dshot (DirectX) | 60-120 FPS | ✅ | ❌ 依赖旧 Pillow | 不可用 |
| MSS (优化) | 30-60 FPS | ❌ | ✅ | **推荐** |
| PIL.ImageGrab | 15-20 FPS | ❌ | ✅ | 备选 |
| win32gui GDI | 30-50 FPS | ❌ | ✅ | 备选 |

## 问题分析

### d3dshot 安装失败原因

```
ERROR: Pillow 7.1.2 does not support Python 3.12
```

d3dshot 依赖 `pillow<7.2.0`，但该版本不支持 Python 3.12。

### DXGI Desktop Duplication 实现难度

需要完整的 COM 接口实现：
- D3D11CreateDevice
- IDXGIDevice->GetAdapter
- IDXGIAdapter->EnumOutputs
- IDXGIOutput->DuplicateOutput
- IDXGIOutputDuplication->AcquireNextFrame
- ID3D11Texture2D->CopyResource
- ID3D11Texture2D->Map

这些需要大量 ctypes 代码和 C 结构体定义。

## 当前最佳方案

### 优化的 MSS 捕获

已测试性能：
- **1920x1080**: 29.6 FPS
- **1280x720**: ~40-50 FPS
- **平均捕获时间**: 33.7 ms

### 优化配置

```python
import mss
import ctypes

# 创建 MSS 实例（全局复用）
sct = mss.mss()

# 计算最优捕获区域（减少数据量）
user32 = ctypes.windll.user32
screen_w = user32.GetSystemMetrics(0)
screen_h = user32.GetSystemMetrics(1)

# 按比例缩放
scale = min(target_width / screen_w, target_height / screen_h)
capture_w = int(screen_w * scale)
capture_h = int(screen_h * scale)

monitor = {
    "left": (screen_w - capture_w) // 2,
    "top": (screen_h - capture_h) // 2,
    "width": capture_w,
    "height": capture_h,
}

# 捕获（高效路径）
screenshot = sct.grab(monitor)
arr = np.frombuffer(screenshot.rgb, dtype=np.uint8)
frame = arr.reshape((capture_h, capture_w, 3))
```

## 硬件编码 + 优化的 MSS 性能

```
当前配置:
  捕获: MSS (优化)
  编码: h264_mf (GPU)

性能:
  捕获 FPS: 29.6 @ 1080p
  编码延迟: 5.4 ms (GPU)
  端到端 FPS: 20.8 (完整流水线)
```

## DXGI 集成的替代方案

### 方案 1: 使用预编译的 d3dshot fork

```bash
# 如果有维护的 fork
pip install git+https://github.com/.../d3dshot
```

### 方案 2: C++ 扩展

创建 Python C 扩展模块封装 DXGI API。

### 方案 3: 等待官方支持

- Windows Graphics Capture API (WinRT)
- 需要	winrt	库支持

### 方案 4: 使用 Rust agent

现有的 agent-rust 已经实现了 DXGI Desktop Duplication。

## 建议的架构

```
┌─────────────────────────────────────────────────────────┐
│                   Python Agent                          │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐         │
│  │ MSS 捕获 │ →  │ h264_mf  │ →  │ RTP 打包 │ → 网络 │
│  │ (30fps)  │    │ (GPU)    │    │          │         │
│  └──────────┘    └──────────┘    └──────────┘         │
│                                                         │
└─────────────────────────────────────────────────────────┘

如果需要更高性能:
┌─────────────────────────────────────────────────────────┐
│                   Rust Agent (已有)                     │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐         │
│  │ DXGI 捕获│ →  │ NVENC    │ →  │ RTP 打包 │ → 网络 │
│  │ (120fps) │    │ (GPU)    │    │          │         │
│  └──────────┘    └──────────┘    └──────────┘         │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

## 结论

1. **Python 环境限制**: d3dshot 和 DXGI ctypes 在 Python 3.12 上都有兼容性问题

2. **当前最佳配置**:
   - MSS 优化捕获 (30 FPS @ 1080p)
   - h264_mf 硬件编码 (5.4ms 延迟)
   - 端到端 20.8 FPS

3. **性能瓶颈**: 捕获 (33.7ms) >> 编码 (5.4ms)
   - 硬件编码器不是瓶颈
   - 要提升性能需要 DXGI 捕获

4. **建议**:
   - 对于演示/测试: 使用 Python Agent (MSS + GPU 编码)
   - 对于生产: 使用 Rust Agent (已有 DXGI + NVENC)

## DXGI 集成代码结构

已创建的文件：

```
src/capture/
├── __init__.py              (已更新，导出 DXGI 类)
├── d3dshot_backend.py       (原有)
├── dxgi_backend.py          (新增，支持 d3dshot/MSS 回退)
└── dxgi_ctypes.py           (新增，ctypes DXGI 尝试)

test_dxgi_capture.py         (测试脚本)
```

如果 d3dshot 可用，`FastDXGICapture` 会自动使用它，否则回退到优化的 MSS。
