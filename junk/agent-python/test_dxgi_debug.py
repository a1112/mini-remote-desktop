"""
测试 DXGI C++ DLL - 调试版本
"""
import ctypes
import sys
import time
from pathlib import Path

sys.path.insert(0, 'src')

# 加载 DLL - 使用绝对路径
dll_path = Path(__file__).parent / 'dxgi_capture.dll'
dll = ctypes.CDLL(str(dll_path))

# 设置函数签名
dll.init_capture.argtypes = [ctypes.c_int]
dll.init_capture.restype = ctypes.c_void_p

dll.capture_frame.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.c_int,
    ctypes.c_void_p,  # info = None for now
]
dll.capture_frame.restype = ctypes.c_int

dll.free_capture.argtypes = [ctypes.c_void_p]
dll.free_capture.restype = None

# 初始化
print("初始化捕获器...")
handle = dll.init_capture(0)
print(f"句柄: {handle}")

if not handle:
    print("初始化失败!")
    sys.exit(1)

# 捕获测试
print("\n尝试捕获...")

# 分配足够大的缓冲区
width, height = 2560, 1440
buffer_size = width * height * 4
buffer = (ctypes.c_ubyte * buffer_size)()

for i in range(10):
    result = dll.capture_frame(handle, buffer, buffer_size, None)
    print(f"  尝试 {i+1}: result = {result}")
    if result == 1:
        print(f"  成功!")
        break
    time.sleep(0.1)

# 清理
dll.free_capture(handle)
print("\n完成")
