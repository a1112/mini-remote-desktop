"""
编译 CUDA kernel 为 PTX 并内嵌到 C++ 代码
"""
import subprocess
import os
import re

# CUDA kernel 源码（优化版本）
cuda_source = r"""
// BGRA → NV12 颜色转换 CUDA kernel
// 使用优化算法提高性能

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
        // 优化：使用整数运算和移位
        // Y = (77*R + 150*G + 29*B + 128) >> 8
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

        // 2x2 采样
        #pragma unroll
        for (int dy = 0; dy < 2; dy++) {
            #pragma unroll
            for (int dx = 0; dx < 2; dx++) {
                if (src_y + dy < height && src_x + dx < width) {
                    int src_idx = (src_y + dy) * src_pitch + (src_x + dx) * 4;

                    uint8_t b = bgra_src[src_idx + 0];
                    uint8_t g = bgra_src[src_idx + 1];
                    uint8_t r = bgra_src[src_idx + 2];

                    // U = -0.169*R - 0.331*G + 0.500*B
                    // V = 0.500*R - 0.419*G - 0.081*B
                    sum_u += (-43 * r - 85 * g + 128 * b + 32768) >> 8;
                    sum_v += (128 * r - 107 * g - 21 * b + 32768) >> 8;
                }
            }
        }

        int dst_idx = y * dst_pitch + x * 2;
        uv_dst[dst_idx + 0] = (sum_u + 2) / 4;  // 平均 4 个像素
        uv_dst[dst_idx + 1] = (sum_v + 2) / 4;
    }
}

// 联合 kernel：同时转换 Y 和 UV（更高效）
__global__ void bgra_to_nv12_kernel(
    const uint8_t* __restrict__ bgra_src,
    uint8_t* __restrict__ y_dst,
    uint8_t* __restrict__ uv_dst,
    int width,
    int height,
    int src_pitch,
    int y_pitch,
    int uv_pitch
) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;

    if (x < width && y < height) {
        // Y 平面
        int src_idx = y * src_pitch + x * 4;
        int y_idx = y * y_pitch + x;

        uint8_t b = bgra_src[src_idx + 0];
        uint8_t g = bgra_src[src_idx + 1];
        uint8_t r = bgra_src[src_idx + 2];

        y_dst[y_idx] = (77 * r + 150 * g + 29 * b + 128) >> 8;

        // UV 平面（每个线程块处理 2x2 区域）
        if ((x & 1) == 0 && (y & 1) == 0) {
            int uv_x = x / 2;
            int uv_y = y / 2;

            if (uv_x < ((width + 1) / 2) && uv_y < ((height + 1) / 2)) {
                int sum_u = 0;
                int sum_v = 0;

                // 收集 2x2 区域的像素
                #pragma unroll
                for (int dy = 0; dy < 2 && y + dy < height; dy++) {
                    #pragma unroll
                    for (int dx = 0; dx < 2 && x + dx < width; dx++) {
                        int idx = (y + dy) * src_pitch + (x + dx) * 4;
                        uint8_t B = bgra_src[idx + 0];
                        uint8_t G = bgra_src[idx + 1];
                        uint8_t R = bgra_src[idx + 2];

                        sum_u += (-43 * R - 85 * G + 128 * B + 32768) >> 8;
                        sum_v += (128 * R - 107 * G - 21 * B + 32768) >> 8;
                    }
                }

                int uv_idx = uv_y * uv_pitch + uv_x * 2;
                uv_dst[uv_idx + 0] = (sum_u + 2) / 4;
                uv_dst[uv_idx + 1] = (sum_v + 2) / 4;
            }
        }
    }
}

}
"""

cpp_dir = r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture"
cuda_file = os.path.join(cpp_dir, "bgra_to_nv12_kernel_opt.cu")
ptx_file = os.path.join(cpp_dir, "bgra_to_nv12_kernel_opt.ptx")
header_file = os.path.join(cpp_dir, "bgra_to_nv12_kernel_ptx_opt.h")

# 写入 CUDA 源码
with open(cuda_file, 'w') as f:
    f.write(cuda_source)

# 编译为 PTX
nvcc_path = r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0\bin\nvcc.exe"
compile_cmd = f'cmd.exe /c "call \\"D:\\Program Files\\Microsoft Visual Studio\\2022\\Community\\VC\\Auxiliary\\Build\\vcvars64.bat\\" && {nvcc_path} -ptx \\"{cuda_file}\\" -o \\"{ptx_file}\\" -I\\\\\"C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v13.0\\\\include\\\\"" 2>&1'

print("Compiling CUDA kernel to PTX...")
result = subprocess.run(compile_cmd, shell=True, capture_output=True, text=True)

if result.returncode == 0:
    print("✓ CUDA kernel PTX compiled successfully")

    # 读取 PTX 文件
    with open(ptx_file, 'r') as f:
        ptx_content = f.read()
    print(f"PTX size: {len(ptx_content)} bytes")

    # 生成 C++ 头文件
    with open(header_file, 'w') as f:
        f.write("// Auto-generated CUDA kernel PTX (Optimized)\n")
        f.write("#pragma once\n\n")
        f.write("namespace {\n")
        f.write("  const char* kernel_ptx = R\"ptx_kernel(\n")
        f.write(ptx_content)
        f.write("\n)ptx_kernel\";\n")
        f.write("}\n")

    print(f"✓ Generated header: {header_file}")
else:
    print(f"✗ CUDA kernel compilation failed:")
    print(result.stderr)
