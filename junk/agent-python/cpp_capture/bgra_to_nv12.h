/**
 * CUDA Kernel: BGRA → NV12 颜色格式转换
 */

#pragma once

#include <cuda_runtime.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * BGRA → NV12 转换 (CUDA kernel)
 *
 * @param bgra_src   输入 BGRA 数据
 * @param nv12_dst   输出 NV12 数据
 * @param width      图像宽度
 * @param height     图像高度
 * @param src_pitch  输入行跨度（字节）
 * @param dst_pitch  输出行跨度（字节）
 * @param stream     CUDA stream
 * @return cudaError_t 错误码
 */
cudaError_t bgra_to_nv12_cuda(
    const uint8_t* bgra_src,
    uint8_t* nv12_dst,
    int width,
    int height,
    int src_pitch,
    int dst_pitch,
    cudaStream_t stream
);

#ifdef __cplusplus
}
#endif
