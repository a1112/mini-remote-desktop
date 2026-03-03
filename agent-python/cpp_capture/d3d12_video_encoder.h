/**
 * D3D12 Video Encoder - Hardware H.264 Encoding
 *
 * 支持:
 * - D3D12 Video Encode API (Windows 11 22H2+)
 * - NVENC via D3D12 interop (NVIDIA GPUs)
 * - 多队列并发流水线
 *
 * 优势:
 * - 零拷贝 (直接使用 D3D12 捕获资源)
 * - GPU 加速
 * - 低延迟
 */
#pragma once

#include <windows.h>
#include <d3d12.h>
#include <d3d12video.h>
#include <dxgi1_6.h>
#include <wrl/client.h>
#include <mutex>
#include <queue>

using Microsoft::WRL::ComPtr;

// 编码配置
struct D3D12EncodeConfig {
    int width;
    int height;
    int framerate;       // FPS
    int bitrate;         // bps
    int gop_size;        // GOP 大小 (关键帧间隔)
    int quality;         // 1-100, 越高越好

    // 编码器类型
    enum EncoderType {
        AUTO = 0,
        D3D12_VIDEO = 1,  // D3D12 Video Encode API
        NVENC = 2,        // NVIDIA NVENC
        AMF = 3,          // AMD AMF
        MF = 4,           // Media Foundation
    } encoder_type;

    // 输出格式
    enum OutputFormat {
        H264 = 0,
        H265 = 1,
        AV1 = 2,
    } output_format;
};

// 编码帧数据
struct D3D12EncodedFrame {
    const unsigned char* data;  // 编码后的数据
    int size;                   // 数据大小
    bool key_frame;             // 是否关键帧
    long long timestamp;        // 时间戳
};

// 编码器句柄
typedef void* HD3D12Encoder;

#ifdef __cplusplus
extern "C" {
#endif

// 导出宏
#ifdef D3D12_VIDEO_ENCODER_EXPORTS
#define ENCODER_API __declspec(dllexport)
#else
#define ENCODER_API __declspec(dllimport)
#endif

/**
 * 初始化编码器
 *
 * @param d3d12_device D3D12 设备指针 (从 capture 获取)
 * @param config 编码配置
 * @return 编码器句柄
 */
ENCODER_API HD3D12Encoder init_d3d12_encoder(void* d3d12_device, const D3D12EncodeConfig* config);

/**
 * 编码一帧
 *
 * @param handle 编码器句柄
 * @param d3d12_resource D3D12 纹理资源 (从 capture 获取)
 * @param timestamp 时间戳
 * @param force_keyframe 是否强制关键帧
 * @return 1 成功, 0 失败
 */
ENCODER_API int encode_d3d12_frame(
    HD3D12Encoder handle,
    void* d3d12_resource,
    long long timestamp,
    int force_keyframe
);

/**
 * 获取编码后的帧
 *
 * @param handle 编码器句柄
 * @param frame 输出帧数据
 * @return 1 有数据, 0 暂无数据
 */
ENCODER_API int get_encoded_frame(
    HD3D12Encoder handle,
    D3D12EncodedFrame* frame
);

/**
 * 释放编码后的帧数据
 *
 * @param frame 帧数据
 */
ENCODER_API void free_encoded_frame(D3D12EncodedFrame* frame);

/**
 * 请求关键帧
 */
ENCODER_API void request_keyframe(HD3D12Encoder handle);

/**
 * 获取编码统计
 */
struct D3D12EncoderStats {
    long long frames_encoded;
    long long bytes_output;
    float current_bitrate;
    float avg_qp;
};

ENCODER_API void get_encoder_stats(HD3D12Encoder handle, D3D12EncoderStats* stats);

/**
 * 释放编码器
 */
ENCODER_API void free_d3d12_encoder(HD3D12Encoder handle);

/**
 * 检查编码器支持
 *
 * @param type 编码器类型
 * @return 1 支持, 0 不支持
 */
ENCODER_API int is_encoder_supported(int type);

/**
 * 获取推荐编码器
 *
 * @return 推荐的编码器类型
 */
ENCODER_API int get_preferred_encoder();

#ifdef __cplusplus
}
#endif


// 内部实现类
class D3D12VideoEncoder {
public:
    D3D12VideoEncoder();
    ~D3D12VideoEncoder();

    bool Initialize(ID3D12Device* device, const D3D12EncodeConfig* config);
    bool Encode(ID3D12Resource* resource, long long timestamp, bool force_keyframe);
    bool GetEncodedFrame(D3D12EncodedFrame* frame);
    void ReleaseFrame(const D3D12EncodedFrame* frame);
    void RequestKeyframe();
    void GetStats(D3D12EncoderStats* stats);
    void Release();

    bool IsInitialized() const { return initialized_; }

private:
    bool InitializeD3D12VideoEncoder();
    bool InitializeNVENC();
    bool CreateEncodeResources();
    bool CreateCommandQueue();
    void EncodeThreadProc();
    void ReorderBitstream();

    // D3D12 组件
    ComPtr<ID3D12Device> device_;
    ComPtr<ID3D12VideoDevice> video_device_;
    ComPtr<ID3D12VideoEncodeCommandList> encode_command_list_;
    ComPtr<ID3D12CommandQueue> encode_queue_;
    ComPtr<ID3D12CommandAllocator> command_allocator_;

    // 编码器资源
    ComPtr<ID3D12VideoEncoder> video_encoder_;
    ComPtr<ID3D12Resource> input_texture_;
    ComPtr<ID3D12Resource> encode_output_;

    // 编码配置
    D3D12EncodeConfig config_;
    D3D12_VIDEO_ENCODER_PROFILE_DESC profile_desc_;
    D3D12_VIDEO_ENCODER_LEVEL_SETTING level_setting_;
    D3D12_VIDEO_ENCODER_RATE_CONTROL rate_control_;

    // 编码统计
    D3D12EncoderStats stats_;
    long long current_pts_;

    // 输出队列
    struct EncodedOutput {
        std::vector<unsigned char> data;
        long long timestamp;
        bool key_frame;
    };
    std::queue<EncodedOutput> output_queue_;
    std::mutex output_mutex_;

    // 状态
    bool initialized_ = false;
    bool encode_thread_running_ = false;
    std::thread encode_thread_;
};
