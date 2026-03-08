"""
创建 CUDA kernel 并编译为 PTX，然后内嵌到 C++ 代码中
"""

import subprocess
import os

# CUDA kernel 源码
cuda_source = r"""
// BGRA → NV12 颜色转换 CUDA kernel

extern "C" {

__global__ void bgra_to_y_kernel(
    const uint8_t* __restrict__ bgra_src,
    uint8_t* __restrict__ y_dst,
    int width,
    int height,
    int src_pitch,
    int dst_pitch
) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;

    if (x < width && y < height) {
        int src_idx = y * src_pitch + x * 4;
        int dst_idx = y * dst_pitch + x;

        uint8_t b = bgra_src[src_idx + 0];
        uint8_t g = bgra_src[src_idx + 1];
        uint8_t r = bgra_src[src_idx + 2];

        // Y = 0.299*R + 0.587*G + 0.114*B
        y_dst[dst_idx] = (77 * r + 150 * g + 29 * b + 128) >> 8;
    }
}

__global__ void bgra_to_uv_kernel(
    const uint8_t* __restrict__ bgra_src,
    uint8_t* __restrict__ uv_dst,
    int width,
    int height,
    int src_pitch,
    int dst_pitch
) {
    int uv_width = (width + 1) / 2;
    int uv_height = (height + 1) / 2;

    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;

    if (x < uv_width && y < uv_height) {
        int src_x = x * 2;
        int src_y = y * 2;

        int sum_u = 0;
        int sum_v = 0;
        int count = 0;

        for (int dy = 0; dy < 2 && src_y + dy < height; dy++) {
            for (int dx = 0; dx < 2 && src_x + dx < width; dx++) {
                int src_idx = (src_y + dy) * src_pitch + (src_x + dx) * 4;

                uint8_t b = bgra_src[src_idx + 0];
                uint8_t g = bgra_src[src_idx + 1];
                uint8_t r = bgra_src[src_idx + 2];

                sum_u += (-43 * r - 85 * g + 128 * b + 32768) >> 8;
                sum_v += (128 * r - 107 * g - 21 * b + 32768) >> 8;
                count++;
            }
        }

        int dst_idx = y * dst_pitch + x * 2;
        uv_dst[dst_idx + 0] = sum_u / count;
        uv_dst[dst_idx + 1] = sum_v / count;
    }
}

}
"""

cpp_dir = r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"
cuda_dir = cpp_dir
cuda_file = os.path.join(cuda_dir, "bgra_to_nv12_kernel.cu")
ptx_file = os.path.join(cuda_dir, "bgra_to_nv12_kernel.ptx")

# 写入 CUDA 源码
with open(cuda_file, 'w') as f:
    f.write(cuda_source)

# 编译为 PTX
nvcc_path = r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin\nvcc.exe"
compile_cmd = f'cmd.exe /c "call \\"D:\\Program Files\\Microsoft Visual Studio\\2022\\Community\\VC\\Auxiliary\\Build\\vcvars64.bat\\" && {nvcc_path} -ptx \\"{cuda_file}\\" -o \\"{ptx_file}\\" -I\\"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.0\\include\\""'

print("Compiling CUDA kernel to PTX...")
result = subprocess.run(compile_cmd, shell=True, capture_output=True, text=True)

if result.returncode == 0:
    print("✓ CUDA kernel PTX compiled successfully")
    # 读取 PTX 文件
    with open(ptx_file, 'r') as f:
        ptx_content = f.read()
    print(f"PTX size: {len(ptx_content)} bytes")

    # 生成 C++ 头文件，包含 PTX 作为字符串字面量
    header_file = os.path.join(cuda_dir, "bgra_to_nv12_kernel_ptx.h")
    with open(header_file, 'w') as f:
        f.write(f"// Auto-generated CUDA kernel PTX\n")
        f.write(f"#pragma once\n\n")
        f.write(f"namespace {{\n")
        f.write(f"  const char* kernel_ptx = R\"ptx_kernel(\n")
        f.write(ptx_content)
        f.write(f"\n)ptx_kernel\";\n")
        f.write(f"}}\n")

    print(f"✓ Generated header: {header_file}")
else:
    print(f"✗ CUDA kernel compilation failed:")
    print(result.stderr)
