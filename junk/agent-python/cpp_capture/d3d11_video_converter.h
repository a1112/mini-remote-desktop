/**
 * D3D11 Video Processor 颜色转换
 * 使用硬件加速进行 BGRA→NV12 转换
 */

#pragma once

#include <d3d11.h>
#include <d3d11_1.h>
#include <d3d11_2.h>
#include <d3d11_3.h>
#include <dxgi1_2.h>
#include <wrl/client.h>
#include <memory>

using Microsoft::WRL::ComPtr;

/**
 * D3D11 视频处理器 - 硬件加速颜色转换
 */
class D3D11VideoConverter {
public:
    D3D11VideoConverter();
    ~D3D11VideoConverter();

    /**
     * 初始化视频处理器
     * @param device D3D11 设备
     * @param width 输入宽度
     * @param height 输入高度
     * @return 是否成功
     */
    bool Initialize(ID3D11Device* device, int width, int height);

    /**
     * 转换 BGRA 纹理到 NV12
     * @param context D3D11 上下文
     * @param src_texture BGRA 源纹理
     * @param dst_y_texture Y 平面目标纹理
     * @param dst_uv_texture UV 平面目标纹理
     * @return 是否成功
     */
    bool Convert(
        ID3D11DeviceContext* context,
        ID3D11Texture2D* src_texture,
        ID3D11Texture2D* dst_y_texture,
        ID3D11Texture2D* dst_uv_texture
    );

    /**
     * 获取 Y 平面输出纹理
     */
    ID3D11Texture2D* GetYTexture() const { return y_texture_.Get(); }

    /**
     * 获取 UV 平面输出纹理
     */
    ID3D11Texture2D* GetUVTexture() const { return uv_texture_.Get(); }

    /**
     * 释放资源
     */
    void Release();

    bool IsInitialized() const { return initialized_; }

private:
    bool CreateViews();
    bool CreateProcessor();

    int width_;
    int height_;

    ComPtr<ID3D11Device> device_;
    ComPtr<ID3D11VideoDevice> video_device_;
    ComPtr<ID3D11VideoContext> video_context_;

    ComPtr<ID3D11VideoProcessor> video_processor_;
    ComPtr<ID3D11VideoProcessorEnumerator> processor_enumerator_;

    ComPtr<ID3D11Texture2D> y_texture_;
    ComPtr<ID3D11Texture2D> uv_texture_;

    ComPtr<ID3D11VideoProcessorInputView> input_view_;
    ComPtr<ID3D11VideoProcessorOutputView> output_view_;

    D3D11_VIDEO_PROCESSOR_CONTENT_DESC content_desc_;
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC output_desc_;

    bool initialized_;
};
