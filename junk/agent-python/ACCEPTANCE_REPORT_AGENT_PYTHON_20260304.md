# Agent-Python 验收报告（分阶段开销）

日期：2026-03-04  
目录：`J:\ProjectTest\远程探查\mini-remote-desktop\agent-python`

## 1. 验收目标

- 验证 `agent-python` 是否可运行。
- 验证 DXGI/WGC + NVENC GPU Direct/ZeroCopy 路径是否打通。
- 输出分阶段开销（采集 / 编码 / 发送）。
- 对照目标 `2K @ 144fps` 给出结论。

## 2. 执行命令与结果

### 2.1 ZeroCopy 路径单测

命令：

```powershell
python -m pytest -q test_nvenc_encoder_gpu_direct_path.py
```

结果：

- `4 passed in 0.16s`
- 说明 Python 侧逻辑满足：
  - 优先调用 `encode_nvenc_frame_d3d11_zerocopy`
  - 失败后回退 `encode_nvenc_frame_d3d11`
  - 首次失败后禁用 zerocopy 重试风暴
  - 初始化优先 `init_nvenc_encoder_d3d11_zerocopy`

### 2.2 采集阶段（WGC）

命令：

```powershell
python test_wgc_nvenc_perf.py --mode monitor
```

关键结果（2560x1440）：

- 实际 FPS：`173.0`
- 捕获延迟平均：`1.541 ms`
- 捕获延迟 P95：`2.246 ms`

结论：采集端性能充足，不是 2K144 的主瓶颈。

### 2.3 编码阶段（NVENC）

命令：

```powershell
python test_gpu_direct_full.py
```

关键结果（2560x1440）：

- 编码延迟平均：`15.54 ms`
- 编码延迟最小/最大：`14.14 / 19.70 ms`
- 脚本给出的总延迟估计：`17.54 ms`（含 ~2ms 捕获）
- 理论 FPS：`57.0`

原生日志要点：

- `Input buffer mode: NV12 buffers`
- `Created intermediate texture for cross-device copy`

结论：当前 native 实际是“GPU 上有中间拷贝/格式转换”路径，不是纯粹端到端零拷贝直通。

### 2.4 发送/传输阶段（Python 管理开销）

命令：

```powershell
python bench_transport.py
python test_e2e_transport.py
```

关键结果：

- `FrameInfo` 构造吞吐：`~1,112,718 fps`
- `Stats` 更新：`~2,934,014 updates/sec`
- `JSON` 编解码：`~287,043 ops/sec`
- `test_e2e_transport.py`：`3/3 PASS`

结论：Python 侧封装/管理开销非常小，不是主瓶颈；真实发送开销更多受网络与协议栈 RTT/丢包影响。

## 3. 当前链路判定

当前可运行链路（实测）：

1. `WGC/DXGI` 捕获（D3D11 纹理）
2. 复制/转换到 NVENC 可接受输入（NV12 相关路径）
3. `NVENC` 编码 H.264
4. 进入传输层（QUIC/WebRTC 适配）

ZeroCopy 现状：

- Python 层“会优先尝试 zerocopy”已验证。
- Native 运行日志显示仍存在中间纹理/转换路径，尚未达到“严格意义纯 zero-copy 直通”。

## 4. 对 2K@144 验收结论

目标：`2560x1440 @ 144fps` 对应帧预算 `6.94 ms/frame`。  
当前关键实测：

- 捕获：`~1.54 ms`
- 编码：`~15.54 ms`
- 采集+编码合计：`~17.08 ms`（未计真实网络发送）

结论：**当前不满足 2K@144**。主瓶颈在编码路径（含纹理/格式处理），不是采集与 Python 管理层。

## 5. 风险与说明

- `test_gpu_direct.py` 在当前版本中大量 `capture failed`，其 2 帧样本结果不具代表性，未纳入最终结论。
- 部分脚本打印分辨率初值 `0x0`，但首帧后恢复真实值；最终统计已基于真实帧。

