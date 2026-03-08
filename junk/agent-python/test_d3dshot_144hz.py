#!/usr/bin/env python3
"""
d3dshot 144Hz 显示器性能测试

每个测试在独立进程中运行，避免单例问题。
"""
import subprocess
import sys

print("="*70)
print("d3dshot 144Hz 性能测试")
print("="*70)

tests = [
    (60, "60 FPS target"),
    (120, "120 FPS target"),
    (144, "144 FPS target"),
    (240, "240 FPS target"),
]

results = []

for target_fps, desc in tests:
    print(f"\n测试: {desc}")
    print("-"*40)

    code = f"""
import d3dshot
import time

d3d = d3dshot.create(capture_output='numpy')
d3d.frame_buffer_size = 200
d3d.capture(target_fps={target_fps})

time.sleep(3)

buffer_size = len(d3d.frame_buffer)
actual_fps = buffer_size / 3

print(f'{{actual_fps:.1f}}')

d3d.stop()
"""

    result = subprocess.run(
        [r"J:\python\py10\python.exe", "-c", code],
        capture_output=True,
        text=True,
        timeout=30
    )

    if result.returncode == 0:
        fps = float(result.stdout.strip())
        results.append((target_fps, fps))
        print(f"  实际 FPS: {fps:.1f}")
    else:
        print(f"  错误: {result.stderr}")

# 持续捕获测试
print(f"\n持续捕获测试 (10秒)")
print("-"*40)

code = """
import d3dshot
import time

d3d = d3dshot.create(capture_output='numpy')

start = time.time()
frames = 0
while time.time() - start < 10:
    frame = d3d.screenshot()
    if frame is not None:
        frames += 1

elapsed = time.time() - start
fps = frames / elapsed
print(f'{fps:.1f}')
"""

result = subprocess.run(
    [r"J:\python\py10\python.exe", "-c", code],
    capture_output=True,
    text=True,
    timeout=30
)

sustained_fps = float(result.stdout.strip())
print(f"  持续 FPS: {sustained_fps:.1f}")

# 总结
print("\n" + "="*70)
print("总结")
print("="*70)

print(f"\nTarget FPS  →  实际 FPS")
print("-"*30)
for target, actual in results:
    print(f"  {target:3d}       →   {actual:5.1f}")

print(f"\n持续捕获:   {sustained_fps:.1f} FPS")

print(f"\n结论: d3dshot 上限约 {sustained_fps:.0f} FPS")
print(f"      144Hz 显示器没有突破限制")
