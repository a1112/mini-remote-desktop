/**
 * CUDA Kernel: BGRA → NV12 颜色格式转换
 *
 * 输入: BGRA 纹理数据 (每像素 4 字节)
 * 输出: NV12 格式 (Y 平面 + 交错 UV 平面)
 *
 * 完全在 GPU 上执行，无需 CPU 参与
 */

#include <cuda_runtime.h>

// Y 分量转换: Y = 0.299*R + 0.587*G + 0.114*B
// 使用整数运算优化: Y = (77*R + 150*G + 29*B) >> 8
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
        // 使用整数运算: (77*R + 150*G + 29*B + 128) >> 8
        y_dst[dst_idx] = (77 * r + 150 * g + 29 * b + 128) >> 8;
    }
}

// UV 分量转换: 下采样 2x2
__global__ void bgra_to_uv_kernel(
    const uint8_t* __restrict__ bgra_src,
    uint8_t* __restrict__ uv_dst,
    int width,
    int height,
    int src_pitch,
    int dst_pitch
) {
    // UV 分辨率是 Y 的一半
    int uv_width = (width + 1) / 2;
    int uv_height = (height + 1) / 2;

    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;

    if (x < uv_width && y < uv_height) {
        // 获取 2x2 像素块的平均值
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

                // U = -0.169*R - 0.331*G + 0.500*B + 128
                // V = 0.500*R - 0.419*G - 0.081*B + 128
                // 整数优化:
                // U = (-43*R - 85*G + 128*B + 32768) >> 8
                // V = (128*R - 107*G - 21*B + 32768) >> 8

                sum_u += (-43 * r - 85 * g + 128 * b + 32768) >> 8;
                sum_v += (128 * r - 107 * g - 21 * b + 32768) >> 8;
                count++;
            }
        }

        // 取平均值
        int dst_idx = y * dst_pitch + x * 2;  // UV 交错存储
        uv_dst[dst_idx + 0] = sum_u / count;  // U
        uv_dst[dst_idx + 1] = sum_v / count;  // V
    }
}

// 组合函数: 完整的 BGRA → NV12 转换
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

    // Y 平面指针
    uint8_t* y_dst = nv12_dst;
    // UV 平面指针
    uint8_t* uv_dst = nv12_dst + dst_pitch * height;

    // 计算线程块大小和网格大小
    dim3 block_size(16, 16);
    dim3 grid_size(
        (width + block_size.x - 1) / block_size.x,
        (height + block_size.y - 1) / block_size.y
    );

    // 执行 Y 分量转换
    bgra_to_y_kernel<<<grid_size, block_size, 0, stream>>>(
        bgra_src, y_dst, width, height, src_pitch, dst_pitch
    );

    // UV 分辨率
    int uv_width = (width + 1) / 2;
    int uv_height = (height + 1) / 2;

    dim3 uv_grid_size(
        (uv_width + block_size.x - 1) / block_size.x,
        (uv_height + block_size.y - 1) / block_size.y
    );

    // 执行 UV 分量转换
    bgra_to_uv_kernel<<<uv_grid_size, block_size, 0, stream>>>(
        bgra_src, uv_dst, width, height, src_pitch, dst_pitch
    );

    return cudaGetLastError();
}

}  // extern "C"
