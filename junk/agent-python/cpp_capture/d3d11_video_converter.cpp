/**
 * D3D11 Video Processor 颜色转换实现
 */

#include "d3d11_video_converter.h"
#include <iostream>

D3D11VideoConverter::D3D11VideoConverter()
    : width_(0)
    , height_(0)
    , initialized_(false)
{
}

D3D11VideoConverter::~D3D11VideoConverter() {
    Release();
}

bool D3D11VideoConverter::Initialize(ID3D11Device* device, int width, int height) {
    if (!device) return false;

    device_ = device;
    width_ = width;
    height_ = height;

    // 获取 D3D11 Video 设备
    HRESULT hr = device_->QueryInterface(__uuidof(ID3D11VideoDevice), &video_device_);
    if (FAILED(hr)) {
        std::cerr << "[D3D11Video] QueryInterface(ID3D11VideoDevice) failed: 0x" << std::hex << hr << std::endl;
        return false;
    }

    // 获取 D3D11 上下文
    ComPtr<ID3D11DeviceContext> context;
    device_->GetImmediateContext(&context);

    hr = context->QueryInterface(__uuidof(ID3D11VideoContext), &video_context_);
    if (FAILED(hr)) {
        std::cerr << "[D3D11Video] QueryInterface(ID3D11VideoContext) failed: 0x" << std::hex << hr << std::endl;
        return false;
    }

    // 创建输出纹理 (Y 平面)
    D3D11_TEXTURE2D_DESC yDesc = {};
    yDesc.Width = width;
    yDesc.Height = height;
    yDesc.MipLevels = 1;
    yDesc.ArraySize = 1;
    yDesc.Format = DXGI_FORMAT_R8_UNORM;  // Y 平面为 8 位灰度
    yDesc.SampleDesc.Count = 1;
    yDesc.Usage = D3D11_USAGE_DEFAULT;
    yDesc.BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;

    hr = device_->CreateTexture2D(&yDesc, nullptr, &y_texture_);
    if (FAILED(hr)) {
        std::cerr << "[D3D11Video] Create Y texture failed: 0x" << std::hex << hr << std::endl;
        return false;
    }

    // 创建输出纹理 (UV 平面)
    D3D11_TEXTURE2D_DESC uvDesc = {};
    uvDesc.Width = (width + 1) / 2;
    uvDesc.Height = (height + 1) / 2;
    uvDesc.MipLevels = 1;
    uvDesc.ArraySize = 1;
    uvDesc.Format = DXGI_FORMAT_R8G8_UNORM;  // UV 平面为 8 位交错
    uvDesc.SampleDesc.Count = 1;
    uvDesc.Usage = D3D11_USAGE_DEFAULT;
    uvDesc.BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;

    hr = device_->CreateTexture2D(&uvDesc, nullptr, &uv_texture_);
    if (FAILED(hr)) {
        std::cerr << "[D3D11Video] Create UV texture failed: 0x" << std::hex << hr << std::endl;
        return false;
    }

    // 设置内容描述
    content_desc_.InputFrameFormat = D3D11_VIDEO_FRAME_FORMAT_B8G8R8A8_UNORM;
    content_desc_.InputFrameRate.Numerator = 60;
    content_desc_.InputFrameRate.Denominator = 1;
    content_desc_.InputWidth = width;
    content_desc_.InputHeight = height;
    content_desc_.OutputFrameRate.Numerator = 60;
    content_desc_.OutputFrameRate.Denominator = 1;
    content_desc_.OutputWidth = width;
    content_desc_.OutputHeight = height;
    content_desc_.Usage = D3D11_VIDEO_USAGE_PLAYBACK_NORMAL;

    // 创建视频处理器枚举器
    hr = video_device_->CreateVideoProcessorEnumerator(
        &content_desc_,
        &processor_enumerator_
    );
    if (FAILED(hr)) {
        std::cerr << "[D3D11Video] CreateVideoProcessorEnumerator failed: 0x" << std::hex << hr << std::endl;
        return false;
    }

    // 获取推荐的渲染目标格式
    UINT rtFormatCount = 0;
    D3D11_VIDEO_PROCESSOR_FORMAT_CAPS formatCaps = {};
    processor_enumerator_->GetVideoProcessorFormatCaps(&formatCaps);
    // 使用 NV12 格式 (R8 + R8G8)

    // 创建视频处理器
    hr = video_device_->CreateVideoProcessor(
        processor_enumerator_.Get(),
        &video_processor_
    );
    if (FAILED(hr)) {
        std::cerr << "[D3D11Video] CreateVideoProcessor failed: 0x" << std::hex << hr << std::endl;
        return false;
    }

    std::cout << "[D3D11Video] Video processor created successfully" << std::endl;
    std::cout << "[D3D11Video] Output textures: Y=" << width << "x" << height
              << " UV=" << uvDesc.Width << "x" << uvDesc.Height << std::endl;

    initialized_ = true;
    return true;
}

bool D3D11VideoConverter::CreateViews() {
    // 创建输入视图
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC inputDesc = {};
    inputDesc.FourCC = 0;  // 不使用 FourCC
    inputDesc.Width = width_;
    inputDesc.Height = height_;
    // 设置其他参数...

    // 创建输出视图
    output_desc_.Width = width_;
    output_desc_.Height = height_;
    output_desc_.Format = D3D11_VIDEO_PROCESSOR_FORMAT_NV12;  // NV12

    return true;
}

bool D3D11VideoConverter::Convert(
    ID3D11DeviceContext* context,
    ID3D11Texture2D* src_texture,
    ID3D11Texture2D* dst_y_texture,
    ID3D11Texture2D* dst_uv_texture
) {
    if (!initialized_) return false;

    // 使用视频处理器进行颜色转换
    // 注意：这里需要正确设置输入/输出视图

    HRESULT hr = video_context_->ProcessVideo(
        video_processor_.Get(),
        0,  // stream index
        1,  // frame count
        input_view_.Get(),
        output_view_.Get()
    );

    return SUCCEEDED(hr);
}

void D3D11VideoConverter::Release() {
    y_texture_.Reset();
    uv_texture_.Reset();
    video_processor_.Reset();
    processor_enumerator_.Reset();
    video_context_.Reset();
    video_device_.Reset();
    device_.Reset();
    initialized_ = false;
}
