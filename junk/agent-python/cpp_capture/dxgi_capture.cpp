/**
 * DXGI Desktop Duplication Capture - Implementation
 */
#include "dxgi_capture.h"
#include <iostream>
#include <vector>

// ============================================================================
// DXGICapturer Implementation
// ============================================================================

DXGICapturer::DXGICapturer() {
}

DXGICapturer::~DXGICapturer() {
    Release();
}

bool DXGICapturer::Initialize(int monitor_index) {
    // 创建 D3D11 设备
    if (!CreateD3DDevice()) {
        return false;
    }

    // 获取输出
    if (!GetOutput(monitor_index)) {
        return false;
    }

    // 创建 Desktop Duplication
    if (!CreateDesktopDupl()) {
        return false;
    }

    // 创建 staging texture
    D3D11_TEXTURE2D_DESC desc = {};
    desc.Width = width_;
    desc.Height = height_;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;

    HRESULT hr = device_->CreateTexture2D(&desc, nullptr, &staging_texture_);
    if (FAILED(hr)) {
        std::cerr << "Failed to create staging texture: " << std::hex << hr << std::endl;
        return false;
    }

    initialized_ = true;
    return true;
}

bool DXGICapturer::CreateD3DDevice() {
    D3D_FEATURE_LEVEL feature_levels[] = {
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    };

    UINT create_flags = 0;
#ifdef _DEBUG
    create_flags |= D3D11_CREATE_DEVICE_DEBUG;
#endif

    D3D_FEATURE_LEVEL selected_level = D3D_FEATURE_LEVEL_10_0;

    HRESULT hr = D3D11CreateDevice(
        nullptr,  // 默认适配器
        D3D_DRIVER_TYPE_HARDWARE,
        nullptr,
        create_flags,
        feature_levels,
        ARRAYSIZE(feature_levels),
        D3D11_SDK_VERSION,
        &device_,
        &selected_level,
        &context_
    );

    if (FAILED(hr)) {
        std::cerr << "D3D11CreateDevice failed: " << std::hex << hr << std::endl;
        return false;
    }

    return true;
}

bool DXGICapturer::GetOutput(int monitor_index) {
    ComPtr<IDXGIDevice> dxgi_device;
    HRESULT hr = device_.As(&dxgi_device);
    if (FAILED(hr)) {
        return false;
    }

    ComPtr<IDXGIAdapter> adapter;
    hr = dxgi_device->GetAdapter(&adapter);
    if (FAILED(hr)) {
        return false;
    }

    // 枚举输出
    ComPtr<IDXGIOutput> output;
    hr = adapter->EnumOutputs(monitor_index, &output);
    if (FAILED(hr)) {
        std::cerr << "EnumOutputs failed for monitor " << monitor_index << std::endl;
        return false;
    }

    DXGI_OUTPUT_DESC desc;
    hr = output->GetDesc(&desc);
    if (FAILED(hr)) {
        return false;
    }

    // 获取桌面坐标
    RECT desktop_rect = desc.DesktopCoordinates;
    width_ = desktop_rect.right - desktop_rect.left;
    height_ = desktop_rect.bottom - desktop_rect.top;

    // 获取 IDXGIOutput1
    ComPtr<IDXGIOutput1> output1;
    hr = output.As(&output1);
    if (FAILED(hr)) {
        std::cerr << "Failed to get IDXGIOutput1" << std::endl;
        return false;
    }

    return true;
}

bool DXGICapturer::CreateDesktopDupl() {
    ComPtr<IDXGIDevice> dxgi_device;
    HRESULT hr = device_.As(&dxgi_device);
    if (FAILED(hr)) {
        return false;
    }

    ComPtr<IDXGIAdapter> adapter;
    hr = dxgi_device->GetAdapter(&adapter);
    if (FAILED(hr)) {
        return false;
    }

    // 枚举输出
    ComPtr<IDXGIOutput> output;
    hr = adapter->EnumOutputs(0, &output);  // 使用索引 0
    if (FAILED(hr)) {
        return false;
    }

    ComPtr<IDXGIOutput1> output1;
    hr = output.As(&output1);
    if (FAILED(hr)) {
        return false;
    }

    // 创建 Desktop Duplication
    hr = output1->DuplicateOutput(device_.Get(), &duplication_);
    if (FAILED(hr)) {
        if (hr == E_ACCESSDENIED) {
            std::cerr << "Desktop Duplication access denied (run as admin?)" << std::endl;
        } else if (hr == ERROR_NOT_SUPPORTED) {
            std::cerr << "Desktop Duplication not supported on this system" << std::endl;
        } else if (hr == E_INVALIDARG) {
            std::cerr << "Desktop Duplication invalid argument" << std::endl;
        } else {
            std::cerr << "DuplicateOutput failed: " << std::hex << hr << std::endl;
        }
        return false;
    }

    return true;
}

bool DXGICapturer::AcquireNextFrame() {
    if (!duplication_) {
        return false;
    }

    // 获取下一帧 (超时 0 = 立即返回)
    HRESULT hr = duplication_->AcquireNextFrame(
        0,  // 毫秒超时
        &frame_info_,
        &resource_
    );

    if (hr == DXGI_ERROR_WAIT_TIMEOUT) {
        // 没有新帧
        return false;
    }

    if (FAILED(hr)) {
        // 可能需要重新初始化 (显示模式改变等)
        if (hr == DXGI_ERROR_ACCESS_LOST) {
            duplication_->ReleaseFrame();
            duplication_.Reset();
            // 重新创建...
        }
        return false;
    }

    // 获取帧纹理
    hr = resource_.As(&frame_texture_);
    if (FAILED(hr)) {
        duplication_->ReleaseFrame();
        return false;
    }

    return true;
}

bool DXGICapturer::CopyFrameToBuffer(unsigned char* buffer, int buffer_size) {
    if (!frame_texture_ || !staging_texture_) {
        return false;
    }

    // 复制到 staging texture
    D3D11_BOX box = {0, 0, 0, width_, height_, 1};
    context_->CopySubresourceRegion(
        staging_texture_.Get(),
        0,
        0, 0, 0,
        frame_texture_.Get(),
        0,
        &box
    );

    // 映射到 CPU
    D3D11_MAPPED_SUBRESOURCE mapped;
    HRESULT hr = context_->Map(
        staging_texture_.Get(),
        0,
        D3D11_MAP_READ,
        0,
        &mapped
    );

    if (FAILED(hr)) {
        return false;
    }

    // 复制数据到缓冲区
    int row_size = width_ * 4;  // BGRA
    int required_size = row_size * height_;

    if (buffer_size < required_size) {
        context_->Unmap(staging_texture_.Get(), 0);
        return false;
    }

    unsigned char* src = static_cast<unsigned char*>(mapped.pData);
    unsigned char* dst = buffer;

    // 逐行复制 (处理 stride)
    for (int y = 0; y < height_; y++) {
        memcpy(dst, src, row_size);
        dst += row_size;
        src += mapped.RowPitch;
    }

    context_->Unmap(staging_texture_.Get(), 0);
    return true;
}

void DXGICapturer::ReleaseFrame() {
    if (duplication_) {
        duplication_->ReleaseFrame();
    }
    resource_.Reset();
    frame_texture_.Reset();
}

bool DXGICapturer::CaptureToBuffer(unsigned char* buffer, int buffer_size, FrameInfo* info) {
    if (!initialized_) {
        return false;
    }

    // 获取下一帧
    if (!AcquireNextFrame()) {
        return false;  // 没有新帧
    }

    // 复制到缓冲区
    bool success = CopyFrameToBuffer(buffer, buffer_size);

    // 填充信息
    if (info && success) {
        info->width = width_;
        info->height = height_;
        info->stride = width_ * 4;
        info->format = DXGI_FORMAT_B8G8R8A8_UNORM;
        info->timestamp = frame_info_.LastPresentTime.QuadPart;
    }

    // 释放帧
    ReleaseFrame();

    return success;
}

void DXGICapturer::Release() {
    duplication_.Reset();
    staging_texture_.Reset();
    resource_.Reset();
    frame_texture_.Reset();
    context_.Reset();
    device_.Reset();
    initialized_ = false;
}

// ============================================================================
// DLL Export Functions
// ============================================================================

extern "C" {

HCaptcha __declspec(dllexport) init_capture(int monitor_index) {
    DXGICapturer* capturer = new DXGICapturer();
    if (!capturer->Initialize(monitor_index)) {
        delete capturer;
        return nullptr;
    }
    return static_cast<HCaptcha>(capturer);
}

int __declspec(dllexport) capture_frame(
    HCaptcha handle,
    unsigned char* buffer,
    int buffer_size,
    FrameInfo* info
) {
    DXGICapturer* capturer = static_cast<DXGICapturer*>(handle);
    if (!capturer || !capturer->IsInitialized()) {
        return 0;
    }

    if (capturer->CaptureToBuffer(buffer, buffer_size, info)) {
        return 1;
    }

    return -1;  // 需要重试 (暂时没新帧)
}

void __declspec(dllexport) free_capture(HCaptcha handle) {
    DXGICapturer* capturer = static_cast<DXGICapturer*>(handle);
    if (capturer) {
        capturer->Release();
        delete capturer;
    }
}

int __declspec(dllexport) get_monitor_count() {
    ComPtr<IDXGIFactory1> factory;
    HRESULT hr = CreateDXGIFactory1(__uuidof(IDXGIFactory1), &factory);
    if (FAILED(hr)) {
        return 0;
    }

    int count = 0;
    ComPtr<IDXGIAdapter1> adapter;
    while (SUCCEEDED(factory->EnumAdapters1(count, &adapter))) {
        ComPtr<IDXGIOutput> output;
        UINT output_index = 0;
        while (SUCCEEDED(adapter->EnumOutputs(output_index, &output))) {
            count++;
            output_index++;
            output.Reset();
        }
        adapter.Reset();
    }

    return count;
}

int __declspec(dllexport) get_monitor_info(int index, int* width, int* height, int* is_primary) {
    // 简化实现
    // 实际应用中需要枚举所有适配器和输出
    return 0;
}

}  // extern "C"
