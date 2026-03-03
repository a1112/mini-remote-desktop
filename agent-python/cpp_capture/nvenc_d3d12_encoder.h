/**
 * D3D12-NVENC 编码器
 *
 * 实现 D3D12 资源直接传递给 NVENC 编码器
 * 通过 CUDA-D3D12 互操作实现零拷贝
 *
 * 依赖:
 * - CUDA Toolkit 11.0+
 * - NVIDIA Video Codec SDK
 * - D3D12
 *
 * 编译:
 *   需要链接: cuda.lib nvcuvid.lib nvEncodeAPI64.lib
 */
#pragma once

#include <windows.h>
#include <d3d12.h>
#include <cuda.h>
#include <nvcuvid.h>
#include <nvEncodeAPI.h>
#include <wrl/client.h>
#include <memory>
#include <vector>

using Microsoft::WRL::ComPtr;

// 编码配置
struct NVENCEncodeConfig {
    int width;
    int height;
    int framerate;
    int bitrate;
    int gop_size;

    // NVENC 特定
    int preset;  // 0=default, 1=slow, 2=medium, 3=fast, 4=fastest
    int rc_mode;  // 0=constqp, 1=vbr, 2=cbr, 4=vbrminqp, 8=cbrld
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
 * 检查 CUDA-D3D12 互操作支持
 */
NVENC_API int is_cuda_d3d12_interop_supported();

/**
 * 初始化 NVENC 编码器
 *
 * @param d3d12_device D3D12 设备指针
 * @param d3d12_queue D3D12 命令队列
 * @param config 编码配置
 * @return 编码器句柄
 */
NVENC_API HNVENCEncoder init_nvenc_encoder(
    void* d3d12_device,
    void* d3d12_queue,
    const NVENCEncodeConfig* config
);

/**
 * 编码一帧 (D3D12 资源)
 *
 * @param handle 编码器句柄
 * @param d3d12_resource D3D12 纹理资源
 * @param timestamp 时间戳
 * @param force_keyframe 是否强制关键帧
 * @return 1 成功, 0 失败
 */
NVENC_API int encode_nvenc_frame_d3d12(
    HNVENCEncoder handle,
    void* d3d12_resource,
    long long timestamp,
    int force_keyframe
);

/**
 * 编码一帧 (CPU 内存)
 *
 * @param handle 编码器句柄
 * @param data RGB 数据
 * @param size 数据大小
 * @param timestamp 时间戳
 * @param force_keyframe 是否强制关键帧
 * @return 1 成功, 0 失败
 */
NVENC_API int encode_nvenc_frame_cpu(
    HNVENCEncoder handle,
    const unsigned char* data,
    int size,
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
 * 获取编码统计
 */
struct NVENCEncoderStats {
    long long frames_encoded;
    long long bytes_output;
    float current_bitrate;
    float avg_qp;
};

NVENC_API void get_nvenc_stats(HNVENCEncoder handle, NVENCEncoderStats* stats);

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

    bool Initialize(ID3D12Device* d3d12_device, ID3D12CommandQueue* d3d12_queue,
                   const NVENCEncodeConfig* config);
    bool EncodeFromD3D12(ID3D12Resource* resource, long long timestamp, bool force_keyframe);
    bool EncodeFromCPU(const unsigned char* data, int size, long long timestamp, bool force_keyframe);
    bool GetEncodedFrame(NVENCEncodedFrame* frame);
    void ReleaseFrame(const NVENCEncodedFrame* frame);
    void RequestKeyframe();
    void GetStats(NVENCEncoderStats* stats);
    void Release();

    bool IsInitialized() const { return initialized_; }

private:
    bool InitializeCUDA();
    bool InitializeNVENC();
    bool RegisterD3D12Resource(ID3D12Resource* resource);
    bool AllocateInputBuffer();
    bool AllocateOutputBuffer();
    void Cleanup();
    void ProcessOutput();

    // D3D12 组件
    ComPtr<ID3D12Device> d3d12_device_;
    ComPtr<ID3D12CommandQueue> d3d12_queue_;

    // CUDA 组件
    CUcontext cuda_context_;
    CUstream cuda_stream_;

    // NVENC 组件
    void* nvenc_encoder_;
    NV_ENCODE_API_FUNCTION_LIST nvenc_api_;

    // 编码配置
    NVENCEncodeConfig config_;

    // 缓冲区
    void* registered_resource_;  // CUDA 注册的 D3D12 资源
    void* input_buffer_;         // NVENC 输入缓冲区
    void* output_buffer_;        // NVENC 输出缓冲区
    int output_buffer_size_;

    // 输出队列
    struct EncodedOutput {
        std::vector<unsigned char> data;
        long long timestamp;
        bool key_frame;
    };
    std::vector<EncodedOutput> output_queue_;
    std::mutex queue_mutex_;

    // 同步
    CUevent encode_event_;
    CUevent copy_event_;

    // 统计
    NVENCEncoderStats stats_;
    long long current_pts_;
    bool force_keyframe_;

    // 状态
    bool initialized_;
    bool cuda_initialized_;
    bool nvenc_initialized_;
};
