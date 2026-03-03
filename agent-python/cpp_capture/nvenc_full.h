/**
 * NVENC 完整编码器
 *
 * 使用 NVENC SDK 13.0 实现完整的硬件编码
 * 支持 D3D11-CUDA 互操作
 */
#pragma once

#include <windows.h>
#include <d3d11.h>
#include <cuda.h>
#include <cuda_runtime.h>
#include <cudaD3D11.h>
#include <wrl/client.h>
#include <memory>
#include <vector>

// NVENC SDK 路径 - 编译时需要
// 注意: 编译时需要添加 /I"J:\ProjectTest\远程探查\mini-remote-desktop\tools\Video_Codec_Interface_13.0.37\Interface"
#include "nvEncodeAPI.h"

using Microsoft::WRL::ComPtr;

// 编码配置
struct NVENCEncodeConfig {
    int width;
    int height;
    int framerate;
    int bitrate;           // 码率 (bps), 用于 CBR/VBR 模式
    int gop_size;
    int preset;           // 0=default, 1=slow, 2=medium, 3=fast, 4=fastest
    int rc_mode;          // 0=constqp, 1=vbr, 2=cbr, 3=cq(保真)
    int quality;          // 质量级别 (1-51), 用于 CQ 模式, 越小质量越高
};

// 编码帧数据
struct NVENCEncodedFrame {
    const unsigned char* data;
    int size;
    bool key_frame;
    long long timestamp;
};

typedef void* HNVENCEncoder;

#ifdef __cplusplus
extern "C" {
#endif

#ifdef NVENC_ENCODER_EXPORTS
#define NVENC_API __declspec(dllexport)
#else
#define NVENC_API __declspec(dllimport)
#endif

/**
 * 检查 NVENC 支持情况
 */
NVENC_API int is_nvenc_supported();

/**
 * 检查 CUDA-D3D11 互操作支持
 */
NVENC_API int is_cuda_d3d11_interop_supported();

/**
 * 初始化 NVENC 编码器 (D3D11 版本)
 */
NVENC_API HNVENCEncoder init_nvenc_encoder_d3d11(
    void* d3d11_device,
    void* d3d11_context,
    const NVENCEncodeConfig* config
);

/**
 * 编码一帧 (CPU 内存 - BGRA 格式)
 * 输入: 宽度×高度×4 字节的 BGRA 数据
 */
NVENC_API int encode_nvenc_frame_cpu(
    HNVENCEncoder handle,
    const unsigned char* data,
    int size,
    long long timestamp,
    int force_keyframe
);

/**
 * 编码一帧 (D3D11 纹理)
 */
NVENC_API int encode_nvenc_frame_d3d11(
    HNVENCEncoder handle,
    void* d3d11_texture,
    long long timestamp,
    int force_keyframe
);

/**
 * 获取编码后的帧
 */
NVENC_API int get_nvenc_encoded_frame(
    HNVENCEncoder handle,
    NVENCEncodedFrame* frame
);

/**
 * 释放编码后的帧数据
 */
NVENC_API void free_nvenc_encoded_frame(NVENCEncodedFrame* frame);

/**
 * 请求关键帧
 */
NVENC_API void request_nvenc_keyframe(HNVENCEncoder handle);

/**
 * 释放编码器
 */
NVENC_API void free_nvenc_encoder(HNVENCEncoder handle);

/**
 * 获取 NVENC 版本信息
 */
struct NVENCVersion {
    int major;
    int minor;
};
NVENC_API void get_nvenc_version(NVENCVersion* version);

#ifdef __cplusplus
}
#endif

// 内部实现类
class NVENCEncoderImpl {
public:
    NVENCEncoderImpl();
    ~NVENCEncoderImpl();

    bool Initialize(ID3D11Device* d3d11_device, ID3D11DeviceContext* d3d11_context,
                   const NVENCEncodeConfig* config);
    bool EncodeFromCPU(const unsigned char* data, int size, long long timestamp, bool force_keyframe);
    bool EncodeFromD3D11(ID3D11Texture2D* texture, long long timestamp, bool force_keyframe);
    bool GetEncodedFrame(NVENCEncodedFrame* frame);
    void RequestKeyframe();
    void Release();
    bool IsInitialized() const { return initialized_; }

private:
    bool InitializeCUDA();
    bool LoadNVENC();
    bool InitializeNVENCSession();
    bool InitializeEncoderParams();
    bool AllocateBuffers();
    bool RegisterD3D11Resource(ID3D11Texture2D* texture);
    void Cleanup();

    // D3D11 组件
    ComPtr<ID3D11Device> d3d11_device_;
    ComPtr<ID3D11DeviceContext> d3d11_context_;

    // CUDA 组件
    CUcontext cuda_context_;
    cudaStream_t cuda_stream_;

    // NVENC 组件
    HMODULE nvenc_dll_;
    void* nvenc_encoder_;
    NV_ENCODE_API_FUNCTION_LIST nvenc_api_;

    // NVENC 资源
    std::vector<void*> input_buffers_;      // NVENC 输入缓冲区
    std::vector<void*> bitstream_buffers_;  // 输出位流缓冲区
    void* registered_resource_;             // 注册的 D3D11 资源
    void* mapped_resource_;                 // 映射的资源

    // 编码配置
    NVENCEncodeConfig config_;
    uint32_t current_input_buffer_;
    uint32_t current_bitstream_buffer_;

    // 输出队列
    struct EncodedOutput {
        std::vector<unsigned char> data;
        long long timestamp;
        bool key_frame;
    };
    std::vector<EncodedOutput> output_queue_;

    // 状态
    bool initialized_;
    bool cuda_initialized_;
    bool nvenc_loaded_;
    long long current_pts_;
    bool force_keyframe_;
};
