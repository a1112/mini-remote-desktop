/**
 * QuickSync (Intel Quick Sync Video) 编码器支持
 * 使用 Intel Media SDK 进行 H.264 硬件编码
 * 支持 BGRA 输入（无需颜色转换！）
 */

#pragma once

#include <windows.h>
#include <d3d11.h>
#include <wrl/client.h>
#include <memory>

// Intel Media SDK 头文件
#include "mfxdefs.h"
#include "mfxstructures.h"
#include "mfxvideo.h"

using Microsoft::WRL::ComPtr;

/**
 * QuickSync 编码配置
 */
struct QSVEncodeConfig {
    int width;
    int height;
    int framerate;
    int bitrate;        // 码率 (bps)
    int gop_size;
    int quality;        // 质量 (0-51)
};

/**
 * QuickSync 编码后的帧
 */
struct QSVEncodedFrame {
    const unsigned char* data;
    int size;
    bool key_frame;
    long long timestamp;
};

typedef void* HQSVEncoder;

#ifdef __cplusplus
extern "C" {
#endif

#ifdef QSV_ENCODER_EXPORTS
#define QSV_API __declspec(dllexport)
#else
#define QSV_API __declspec(dllimport)
#endif

/**
 * 检查 QuickSync 支持
 */
QSV_API int qsv_is_supported();

/**
 * 检查 D3D11 互操作支持
 */
QSV_API int qsv_is_d3d11_interop_supported();

/**
 * 初始化 QuickSync 编码器
 */
QSV_API HQSVEncoder qsv_create_encoder_d3d11(
    void* d3d11_device,
    void* d3d11_context,
    const QSVEncodeConfig* config
);

/**
 * 编码一帧 (D3D11 纹理)
 * QuickSync 支持 BGRA 输入！
 */
QSV_API int qsv_encode_frame_d3d11(
    HQSVEncoder handle,
    void* d3d11_texture,
    long long timestamp,
    int force_keyframe
);

/**
 * 获取编码后的帧
 */
QSV_API int qsv_get_encoded_frame(
    HQSVEncoder handle,
    QSVEncodedFrame* frame
);

/**
 * 释放编码器
 */
QSV_API void qsv_free_encoder(HQSVEncoder handle);

#ifdef __cplusplus
}
#endif

/**
 * QuickSync 编码器实现类
 */
class QSVEncoderImpl {
public:
    QSVEncoderImpl();
    ~QSVEncoderImpl();

    bool Initialize(ID3D11Device* d3d11_device, ID3D11DeviceContext* d3d11_context,
                   const QSVEncodeConfig* config);
    bool EncodeFromD3D11(ID3D11Texture2D* texture, long long timestamp, bool force_keyframe);
    bool GetEncodedFrame(QSVEncodedFrame* frame);
    void Release();
    bool IsInitialized() const { return initialized_; }

private:
    bool LoadMediaSDK();
    bool CreateSession();
    bool CreateEncoder();
    void Cleanup();

    // D3D11 组件
    ComPtr<ID3D11Device> d3d11_device_;
    ComPtr<ID3D11DeviceContext> d3d11_context_;

    // Media SDK 组件
    mfxSession mfx_session_;
    mfxVideoParam mfx_params_;

    // 编码配置
    QSVEncodeConfig config_;

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
    int frame_count_;
};
