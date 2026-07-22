
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
