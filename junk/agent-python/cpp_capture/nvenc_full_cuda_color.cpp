/**
 * NVENC 编码器 - 带 CUDA kernel 颜色转换优化
 * 使用 CUDA Runtime API 实现 GPU 端 BGRA→NV12 转换
 */

#include "nvenc_full.h"
#include <cuda_runtime.h>
#include <device_launch_parameters.h>

// CUDA kernel 函数声明
extern "C" {
    // Y = 0.299*R + 0.587*G + 0.114*B = (77*R + 150*G + 29*B + 128) >> 8
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
            for (int dy = 0; dy < 2 && src_y + dy < height; dy++) {
                for (int dx = 0; dx < 2 && src_x + dx < width; dx++) {
                    int src_idx = (src_y + dy) * src_pitch + (src_x + dx) * 4;
                    uint8_t b = bgra_src[src_idx + 0];
                    uint8_t g = bgra_src[src_idx + 1];
                    uint8_t r = bgra_src[src_idx + 2];

                    sum_u += (-43 * r - 85 * g + 128 * b + 32768) >> 8;
                    sum_v += (128 * r - 107 * g - 21 * b + 32768) >> 8;
                }
            }

            int dst_idx = y * dst_pitch + x * 2;
            uv_dst[dst_idx + 0] = sum_u / 4;
            uv_dst[dst_idx + 1] = sum_v / 4;
        }
    }
}

namespace {

// CUDA kernel 颜色转换辅助类
class CudaColorConverter {
public:
    static bool ConvertBGRAtoNV12(
        const uint8_t* d_bgra_src,  // Device pointer
        uint8_t* d_y_dst,            // Device pointer
        uint8_t* d_uv_dst,           // Device pointer
        int width,
        int height,
        int src_pitch,
        int y_pitch,
        int uv_pitch,
        cudaStream_t stream
    ) {
        // Y 平面转换
        dim3 blockSize(16, 16);
        dim3 gridSize(
            (width + blockSize.x - 1) / blockSize.x,
            (height + blockSize.y - 1) / blockSize.y
        );

        bgra_to_y_kernel<<<gridSize, blockSize, 0, stream>>>(
            d_bgra_src, d_y_dst, width, height, src_pitch, y_pitch
        );

        // UV 平面转换
        dim3 uvBlockSize(16, 16);
        dim3 uvGridSize(
            ((width + 1) / 2 + uvBlockSize.x - 1) / uvBlockSize.x,
            ((height + 1) / 2 + uvBlockSize.y - 1) / uvBlockSize.y
        );

        bgra_to_uv_kernel<<<uvGridSize, uvBlockSize, 0, stream>>>(
            d_bgra_src, d_uv_dst, width, height, src_pitch, uv_pitch
        );

        return cudaGetLastError() == cudaSuccess;
    }
};

} // namespace

// 导出函数供 NVENCEncoderImpl 使用
extern "C" {

/**
 * 使用 CUDA kernel 进行 BGRA→NV12 颜色转换
 * 所有操作在 GPU 上完成
 */
int cuda_bgra_to_nv12(
    const void* d_bgra_src,   // Device pointer to BGRA source
    void* d_y_dst,             // Device pointer to Y destination
    void* d_uv_dst,            // Device pointer to UV destination
    int width,
    int height,
    int src_pitch,
    int y_pitch,
    int uv_pitch,
    void* stream               // cudaStream_t
) {
    return CudaColorConverter::ConvertBGRAtoNV12(
        (const uint8_t*)d_bgra_src,
        (uint8_t*)d_y_dst,
        (uint8_t*)d_uv_dst,
        width, height, src_pitch, y_pitch, uv_pitch,
        (cudaStream_t)stream
    ) ? 1 : 0;
}

} // extern "C"
