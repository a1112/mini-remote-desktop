/**
 * AMF (AMD Media Framework) 编码器支持
 * 支持 VCE (Video Coding Engine) 硬件编码
 */

#pragma once

#include <windows.h>
#include <d3d11.h>
#include <wrl/client.h>
#include <memory>

// AMF SDK 头文件路径
// 需要安装 AMD AMF SDK
#include <AMF/core/AMF.h>
#include <AMF/components/VideoEncoderVCE/VideoEncoderVCE.h>
#include <AMF/components/VideoDecoder/VideoDecoder.h>

using Microsoft::WRL::ComPtr;

/**
 * AMF 编码配置
 */
struct AMFEncodeConfig {
    int width;
    int height;
    int framerate;
    int bitrate;        // 码率 (bps)
    int gop_size;
    int quality;        // 质量 (0-51, 越小越好)
};

/**
 * AMF 编码后的帧
 */
struct AMFEncodedFrame {
    const unsigned char* data;
    int size;
    bool key_frame;
    long long timestamp;
};

typedef void* HAMFEncoder;

#ifdef __cplusplus
extern "C" {
#endif

#ifdef AMF_ENCODER_EXPORTS
#define AMF_API __declspec(dllexport)
#else
#define AMF_API __declspec(dllimport)
#endif

/**
 * 检查 AMF 支持
 */
AMF_API int amf_is_supported();

/**
 * 检查 AMF-D3D11 互操作支持
 */
AMF_API int amf_is_d3d11_interop_supported();

/**
 * 初始化 AMF 编码器
 */
AMF_API HAMFEncoder amf_create_encoder_d3d11(
    void* d3d11_device,
    void* d3d11_context,
    const AMFEncodeConfig* config
);

/**
 * 编码一帧 (D3D11 纹理)
 * AMF 支持 BGRA 输入，可能比 NVENC 更友好
 */
AMF_API int amf_encode_frame_d3d11(
    HAMFEncoder handle,
    void* d3d11_texture,
    long long timestamp,
    int force_keyframe
);

/**
 * 获取编码后的帧
 */
AMF_API int amf_get_encoded_frame(
    HAMFEncoder handle,
    AMFEncodedFrame* frame
);

/**
 * 释放编码器
 */
AMF_API void amf_free_encoder(HAMFEncoder handle);

#ifdef __cplusplus
}
#endif

/**
 * AMF 编码器实现类
 */
class AMFEncoderImpl {
public:
    AMFEncoderImpl();
    ~AMFEncoderImpl();

    bool Initialize(ID3D11Device* d3d11_device, ID3D11DeviceContext* d3d11_context,
                   const AMFEncodeConfig* config);
    bool EncodeFromD3D11(ID3D11Texture2D* texture, long long timestamp, bool force_keyframe);
    bool GetEncodedFrame(AMFEncodedFrame* frame);
    void Release();
    bool IsInitialized() const { return initialized_; }

private:
    bool LoadAMF();
    bool CreateContext();
    bool CreateEncoder();
    void Cleanup();

    // D3D11 组件
    ComPtr<ID3D11Device> d3d11_device_;
    ComPtr<ID3D11DeviceContext> d3d11_context_;

    // AMF 组件
    AMFContext* amf_context_;
    AMFComponent* amf_encoder_;

    // 编码配置
    AMFEncodeConfig config_;

    // 输出队列
    struct EncodedOutput {
        std::vector<unsigned char> data;
        long long timestamp;
        bool key_frame;
    };
    std::vector<EncodedOutput> output_queue_;

    // 状态
    bool initialized_;
    long long current_pts_;
};
