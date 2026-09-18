# NVENC Preset 优化分析报告

## 测试环境
- 分辨率: 1920x1080
- 目标帧率: 60 FPS
- 测试帧数: 300
- GPU: NVIDIA (NVENC)

## Preset 性能对比

| Preset | Tuning | Profile | Bitrate | 平均延迟 | P95 延迟 | 吞吐量 |
|--------|--------|---------|---------|----------|----------|--------|
| **P1 Max Speed** | UltraLowLatency | Baseline | 5 Mbps | **5.75ms** | 6.66ms | **173.8 FPS** |
| P3 Default | LowLatency | High | 12 Mbps | 5.80ms | **6.26ms** | **172.5 FPS** |
| P6 Ultra Low Latency | UltraLowLatency | High | 12 Mbps | 7.87ms | 9.78ms | 127.1 FPS |
| P6 High Refresh Rate | UltraLowLatency | Baseline | 8 Mbps | 8.87ms | 10.87ms | 112.7 FPS |
| P6 Extreme Low Latency | UltraLowLatency | Baseline | 5 Mbps | 8.29ms | 8.76ms | 120.7 FPS |

## 关键发现

### 1. Preset 编号与速度关系
**P1 < P3 < P6 < P7** (速度递减，质量递增)

Preset 定义的是编码器的"速度/质量"平衡点，编号越小速度越快：
- P1: 最快速度，最低质量
- P3: 平衡 (默认)
- P6: 最佳质量，最慢
- P7: 最高质量，极慢

### 2. Tuning 模式 vs Preset
- **LowLatency / UltraLowLatency**: tuning 模式，调整编码策略
- **P1/P3/P6/P7**: preset，定义速度/质量平衡

UltraLowLatency tuning 配合 P6 preset 并不一定最快，因为 P6 的质量设置更高。

### 3. Profile 影响
- **Baseline**: 更简单的编码工具集，编码/解码更快
- **High**: 更多编码工具，质量更好但更慢

### 4. Bitrate 影响
- 更低码率 → 更快编码，但质量下降
- P1 (5Mbps) vs P3 (12Mbps): 延迟相近，吞吐量相近

## QUIC LAN 优化建议

### 当前瓶颈分析 (stress.transport.180s)
```
fps_target: 60
fps_observed: 22.58  (仅 37%)
encode_total_p95_ms: 19.08ms
```

**问题**: 19ms 编码延迟只能支撑 ~52fps 理论值，远低于 60fps 目标。

### 优化方案

#### 方案 A: 使用 P1 Max Speed Preset (推荐)
```rust
NvencH264Encoder::new_max_speed(width, height, fps)
```
- 预期延迟: 5.75ms (P95: 6.66ms)
- 预期吞吐: 170+ FPS
- 优势: 最低延迟、最高吞吐、最低码率
- 劣势: 画质略低

#### 方案 B: 优化码率配置
当前码率可能过高：
```
bitrate_kbps: 849975  (~850Mbps 异常高)
keyframes: 92  (每秒 1.5 个关键帧)
```

建议：
- 降低码率至 5-8 Mbps
- 增加 GOP 间隔 (当前 30 帧可增加到 60)
- 使用 VBR 而非 CBR

#### 方案 C: 启用 D3D11 共享纹理零拷贝
当前可能存在 CPU 拷贝开销：
```rust
// 确保使用共享纹理路径
if let Some(shared) = frame.d3d11_shared_bgra() {
    return self.encode_shared_bgra(frame, shared);
}
```

#### 方案 D: 调整异步编码槽
当前 H264_SHARED_ASYNC_SLOT_COUNT = 2，可考虑：
- 增加到 3-4 以提高吞吐
- 但会增加端到端延迟

### 预期效果

采用 P1 Max Speed 预设后：
- 编码延迟: 19ms → **6ms** (降低 68%)
- 最大吞吐: 52fps → **170fps** (提升 227%)
- 端到端延迟: <10ms (理论)

## 完整优化链路

```
DXGI 捕获 (0ms) → NVENC P1 编码 (6ms) → QUIC 传输 (0.03ms) →
NVDEC 解码 (2-4ms) → D3D11 共享纹理渲染 (<2ms)

总延迟: <12ms
最大吞吐: 170+ FPS @ 1080p
```

## 实施建议

1. **立即**: 在 stress 测试中验证 P1 preset 效果
2. **短期**: 添加可配置 preset 选项到 benchmark 配置
3. **长期**: 实现自适应 preset 选择 (根据 FPS 目标自动调整)
