/**
 * CUDA 内联实现: BGRA → NV12 转换
 *
 * 使用 CUDA Runtime API 在 GPU 上完成转换
 * 不需要单独编译 .cu 文件
 */

#include "bgra_to_nv12.h"
#include <cuda_runtime.h>
#include <device_launch_parameters.h>

// CUDA kernel 定义 - 直接在 .cpp 文件中
// 注意: 这需要在启用 CUDA 的编译器中编译
// 如果使用 MSVC，需要 CUDA 扩展支持

#ifdef __CUDACC__

// Y 分量转换
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

// UV 分量转换
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

extern "C" {

cudaError_t bgra_to_nv12_cuda(
    const uint8_t* bgra_src,
    uint8_t* nv12_dst,
    int width,
    int height,
    int src_pitch,
    int dst_pitch,
    cudaStream_t stream
) {
    if (!bgra_src || !nv12_dst) {
        return cudaErrorInvalidDevicePointer;
    }

    uint8_t* y_dst = nv12_dst;
    uint8_t* uv_dst = nv12_dst + dst_pitch * height;

    dim3 block(16, 16);
    dim3 grid(
        (width + block.x - 1) / block.x,
        (height + block.y - 1) / block.y
    );

    bgra_to_y_kernel<<<grid, block, 0, stream>>>(
        bgra_src, y_dst, width, height, src_pitch, dst_pitch
    );

    int uv_width = (width + 1) / 2;
    int uv_height = (height + 1) / 2;

    dim3 uv_grid(
        (uv_width + block.x - 1) / block.x,
        (uv_height + block.y - 1) / block.y
    );

    bgra_to_uv_kernel<<<uv_grid, block, 0, stream>>>(
        bgra_src, uv_dst, width, height, src_pitch, dst_pitch
    );

    return cudaGetLastError();
}

}  // extern "C"

#endif  // __CUDACC__
