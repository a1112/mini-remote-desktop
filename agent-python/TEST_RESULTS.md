# 实机测试结果总结

## 测试环境
- 系统: Windows 11 Pro for Workstations
- CPU: 20 逻辑核心
- 屏幕: 2560x1440 原生分辨率
- Python: 3.12

## 测试结果对比

### 纯捕获性能 (无显示)

| 捕获方法 | 分辨率 | FPS | 平均帧时间 |
|---------|-------|-----|-----------|
| **GDI (DC复用)** | 1920x1080 | **60.5** | 16.6 ms |
| MSS | 1920x1080 | 30.5 | 32.8 ms |
| PIL.ImageGrab | 1920x1080 | 24.5 | 40.8 ms |
| GDI | 2560x1440 | 30.0 | 33.4 ms |
| MSS | 2560x1440 | ~18 | ~55 ms |

### 实时显示性能 (带 OpenCV 显示)

| 捕获方法 | 分辨率 | FPS | 评级 |
|---------|-------|-----|------|
| **MSS** | 1920x1080 | **20.8** | ⭐ Fair |
| GDI | 1920x1080 | 19.2 | ⭐ Fair |
| MSS | 2560x1440 | 18.1 | ⭐ Fair |
| PIL.ImageGrab | 2560x1440 | 18.7 | ⭐ Fair |

## 性能分析

### 为什么显示后 FPS 下降？

1. **纯捕获** (60 FPS): 只做 BitBlt 操作
2. **带显示** (20 FPS):
   - GetDIBits 拷贝数据
   - numpy 数组转换
   - OpenCV imshow 渲染

### 瓶颈分解

```
纯捕获:
  BitBlt: ~16ms (60 FPS)

带显示:
  BitBlt: ~16ms
  GetDIBits: ~10ms
  numpy 转换: ~5ms
  OpenCV 显示: ~20ms
  总计: ~51ms (19.6 FPS)
```

## 结论

### 捕获后端选择

1. **GDI** - 最佳性能
   - ⭐⭐⭐ 纯捕获: 60 FPS @1080p
   - ⭐⭐ 带显示: 19 FPS @1080p
   - 优点: 无需额外依赖，pywin32 自带

2. **MSS** - 跨平台
   - ⭐⭐ 纯捕获: 30 FPS @1080p
   - ⭐⭐ 带显示: 21 FPS @1080p
   - 优点: Linux/Mac/Windows 通用

3. **PIL.ImageGrab** - 兼容性最好
   - ⭐ 纯捕获: 24 FPS @1080p
   - 优点: 纯 Python，最简单

### 远程桌面建议配置

```json
{
  "capture": {
    "backend": "gdi",
    "target_width": 1920,
    "target_height": 1080,
    "fps": 30
  }
}
```

**预期性能**:
- 捕获: 60 FPS
- 编码: ~50 FPS (libx264 ultrafast)
- 端到端: ~30 FPS 实际输出

## 测试文件

| 文件 | 功能 |
|------|------|
| `test_gdi_reuse.py` | 纯 GDI 性能测试 |
| `test_live_final.py` | MSS 实时显示 |
| `test_live_1080p.py` | 1080p 降缩显示 |
| `test_live_gdi.py` | GDI 实时显示 |

## 运行方式

```bash
# 纯捕获性能测试
python test_gdi_reuse.py

# 实时显示测试 (MSS)
python test_live_final.py --backend mss

# 实时显示测试 (GDI)
python test_live_gdi.py --width 1920 --height 1080
```
