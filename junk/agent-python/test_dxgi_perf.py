"""
DXGI C++ DLL 性能测试
"""
import ctypes
import sys
import time
import numpy as np
from pathlib import Path

sys.path.insert(0, 'src')

# 加载 DLL
dll_path = Path(__file__).parent / 'dxgi_capture.dll'
dll = ctypes.CDLL(str(dll_path))

# 设置函数签名
dll.init_capture.argtypes = [ctypes.c_int]
dll.init_capture.restype = ctypes.c_void_p

class FrameInfo(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("stride", ctypes.c_int),
        ("format", ctypes.c_ulong),
        ("timestamp", ctypes.c_ulonglong),
    ]

dll.capture_frame.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.c_int,
    ctypes.POINTER(FrameInfo)
]
dll.capture_frame.restype = ctypes.c_int

dll.free_capture.argtypes = [ctypes.c_void_p]
dll.free_capture.restype = None

print("="*70)
print("DXGI C++ DLL 性能测试")
print("="*70)

# 初始化
print("\n初始化...")
handle = dll.init_capture(0)
print(f"句柄: {handle}")

if not handle:
    print("初始化失败!")
    sys.exit(1)

# 获取尺寸
print("\n获取帧尺寸...")
width, height = 2560, 1440
buffer_size = width * height * 4
buffer = (ctypes.c_ubyte * buffer_size)()
info = FrameInfo()

result = dll.capture_frame(handle, buffer, buffer_size, ctypes.byref(info))
if result == 1:
    width = info.width
    height = info.height
    print(f"尺寸: {width}x{height}")
else:
    print("无法获取尺寸")
    dll.free_capture(handle)
    sys.exit(1)

# 性能测试
print(f"\n性能测试 (10秒)...")
print("捕获中...")

times = []
frames = 0
start = time.time()

while time.time() - start < 10:
    t0 = time.perf_counter()
    result = dll.capture_frame(handle, buffer, buffer_size, ctypes.byref(info))
    t1 = time.perf_counter()

    if result == 1:
        frames += 1
        times.append((t1 - t0) * 1000)
    elif result == -1:
        # 暂时没有新帧
        pass

elapsed = time.time() - start
fps = frames / elapsed

print(f"\n结果:")
print(f"  捕获帧数: {frames}")
print(f"  FPS: {fps:.1f}")

if times:
    print(f"  平均延迟: {sum(times)/len(times):.2f} ms")
    print(f"  最快延迟: {min(times):.2f} ms")
    print(f"  最慢延迟: {max(times):.2f} ms")
    print(f"  P50 延迟: {sorted(times)[len(times)//2]:.2f} ms")

# 清理
dll.free_capture(handle)

# 对比
print(f"\n" + "="*70)
print(f"性能对比")
print(f"="*70)
print(f"""
  DXGI C++:   {fps:6.1f} FPS  🚀🚀🚀
  d3dshot:    ~60.0  FPS  🚀🚀
  MSS:        ~30.0  FPS  ⚡
""")
