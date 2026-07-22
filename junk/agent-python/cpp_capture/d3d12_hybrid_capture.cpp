/**
 * D3D12 混合捕获实现
 *
 * D3D11 Desktop Duplication + D3D12 输出
 *
 * 更新: 使用 DXGI 1.5+ DuplicateOutput1 API 支持多并发捕获器
 *       可以与 Windows Game Bar 等应用共存
 */

// 定义导出
#define D3D12_HYBRID_CAPTURE_EXPORTS

// 先包含头文件 (在 extern "C" 之前)
#include "d3d12_hybrid_capture.h"

// 确保 DXGI 1.6 可用 (需要 Windows 10 SDK)
#include <dxgi1_6.h>

#include <iostream>
#include <vector>

// 实现 DLL 导出函数
extern "C" {

HD3D12HybridCapture init_hybrid_capture(int monitor_index, int enable_d3d12) {
    D3D12HybridCapturer* capturer = new D3D12HybridCapturer();
    if (!capturer->Initialize(monitor_index, enable_d3d12 != 0)) {
        delete capturer;
        return nullptr;
    }
    return static_cast<HD3D12HybridCapture>(capturer);
}

int capture_hybrid_frame(
    HD3D12HybridCapture handle,
    D3D12HybridFrame* frame_info
) {
    D3D12HybridCapturer* capturer = static_cast<D3D12HybridCapturer*>(handle);
    if (!capturer || !capturer->IsInitialized()) {
        return 0;
    }

    if (capturer->Capture(frame_info)) {
        return 1;
    }

    return -1;
}

int copy_hybrid_frame_to_cpu(
    HD3D12HybridCapture handle,
    unsigned char* buffer,
    int buffer_size
) {
    D3D12HybridCapturer* capturer = static_cast<D3D12HybridCapturer*>(handle);
    if (!capturer || !capturer->IsInitialized()) {
        return 0;
    }

    return capturer->CopyToCPU(buffer, buffer_size) ? 1 : 0;
}

void* get_hybrid_d3d12_device(HD3D12HybridCapture handle) {
    D3D12HybridCapturer* capturer = static_cast<D3D12HybridCapturer*>(handle);
    return capturer ? capturer->GetD3D12Device() : nullptr;
}

void* get_hybrid_d3d12_queue(HD3D12HybridCapture handle) {
    D3D12HybridCapturer* capturer = static_cast<D3D12HybridCapturer*>(handle);
    return capturer ? capturer->GetD3D12Queue() : nullptr;
}

void* get_hybrid_d3d11_resource(HD3D12HybridCapture handle) {
    D3D12HybridCapturer* capturer = static_cast<D3D12HybridCapturer*>(handle);
    return capturer ? capturer->GetD3D11Resource() : nullptr;
}

void* get_hybrid_d3d11_device(HD3D12HybridCapture handle) {
    D3D12HybridCapturer* capturer = static_cast<D3D12HybridCapturer*>(handle);
    return capturer ? capturer->GetD3D11Device() : nullptr;
}

void* get_hybrid_d3d11_context(HD3D12HybridCapture handle) {
    D3D12HybridCapturer* capturer = static_cast<D3D12HybridCapturer*>(handle);
    return capturer ? capturer->GetD3D11Context() : nullptr;
}

void free_hybrid_capture(HD3D12HybridCapture handle) {
    D3D12HybridCapturer* capturer = static_cast<D3D12HybridCapturer*>(handle);
    if (capturer) {
        capturer->Release();
        delete capturer;
    }
}

}  // extern "C"


// ============================================================================
// D3D12HybridCapturer Implementation
// ============================================================================

D3D12HybridCapturer::D3D12HybridCapturer()
    : shared_handle_(nullptr) {
}

D3D12HybridCapturer::~D3D12HybridCapturer() {
    Release();
}

bool D3D12HybridCapturer::Initialize(int monitor_index, bool enable_d3d12) {
    d3d12_enabled_ = enable_d3d12;

    // 创建 D3D11 设备 (必需)
    if (!CreateD3D11Device()) {
        std::cerr << "Failed to create D3D11 device" << std::endl;
        return false;
    }

    // 初始化 Desktop Duplication
    if (!InitializeDesktopDuplication()) {
        std::cerr << "Failed to initialize Desktop Duplication" << std::endl;
        return false;
    }

    // 可选: 创建 D3D12 组件
    if (d3d12_enabled_) {
        if (!CreateD3D12Device()) {
            std::cerr << "Failed to create D3D12 device, continuing with D3D11 only" << std::endl;
            d3d12_enabled_ = false;
        } else if (!CreateSharedResources()) {
            std::cerr << "Failed to create shared resources" << std::endl;
            d3d12_enabled_ = false;
        }
    }

    initialized_ = true;
    return true;
}

bool D3D12HybridCapturer::CreateD3D11Device() {
    D3D_FEATURE_LEVEL feature_levels[] = {
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
    };

    D3D_FEATURE_LEVEL selected_level;

    HRESULT hr = D3D11CreateDevice(
        nullptr,
        D3D_DRIVER_TYPE_HARDWARE,
        nullptr,
        0,
        feature_levels,
        ARRAYSIZE(feature_levels),
        D3D11_SDK_VERSION,
        &d3d11_device_,
        &selected_level,
        &d3d11_context_
    );

    if (FAILED(hr)) {
        std::cerr << "D3D11CreateDevice failed: " << std::hex << hr << std::endl;
        return false;
    }

    return true;
}

bool D3D12HybridCapturer::CreateD3D12Device() {
    // 创建 DXGI 工厂
    ComPtr<IDXGIFactory4> factory;
    HRESULT hr = CreateDXGIFactory1(__uuidof(factory), &factory);
    if (FAILED(hr)) {
        return false;
    }

    // 获取 D3D11 设备的 DXGI 适配器
    ComPtr<IDXGIDevice> dxgi_device;
    hr = d3d11_device_.As(&dxgi_device);
    if (FAILED(hr)) {
        return false;
    }

    ComPtr<IDXGIAdapter> adapter;
    hr = dxgi_device->GetAdapter(&adapter);
    if (FAILED(hr)) {
        return false;
    }

    // 创建 D3D12 设备 (使用同一个适配器)
    hr = D3D12CreateDevice(
        adapter.Get(),
        D3D_FEATURE_LEVEL_11_0,
        __uuidof(d3d12_device_),
        &d3d12_device_
    );

    if (FAILED(hr)) {
        std::cerr << "D3D12CreateDevice failed: " << std::hex << hr << std::endl;
        return false;
    }

    // 创建命令队列
    D3D12_COMMAND_QUEUE_DESC queue_desc = {};
    queue_desc.Type = D3D12_COMMAND_LIST_TYPE_COPY;
    queue_desc.Flags = D3D12_COMMAND_QUEUE_FLAG_NONE;

    hr = d3d12_device_->CreateCommandQueue(&queue_desc, __uuidof(d3d12_queue_), &d3d12_queue_);
    if (FAILED(hr)) {
        return false;
    }

    // 创建命令分配器
    hr = d3d12_device_->CreateCommandAllocator(
        D3D12_COMMAND_LIST_TYPE_COPY,
        __uuidof(d3d12_allocator_),
        &d3d12_allocator_
    );
    if (FAILED(hr)) {
        return false;
    }

    // 创建命令列表
    hr = d3d12_device_->CreateCommandList(
        0,
        D3D12_COMMAND_LIST_TYPE_COPY,
        d3d12_allocator_.Get(),
        nullptr,
        __uuidof(d3d12_list_),
        &d3d12_list_
    );
    if (FAILED(hr)) {
        return false;
    }

    // 关闭命令列表
    d3d12_list_->Close();

    return true;
}

bool D3D12HybridCapturer::InitializeDesktopDuplication() {
    ComPtr<IDXGIDevice> dxgi_device;
    HRESULT hr = d3d11_device_.As(&dxgi_device);
    if (FAILED(hr)) {
        return false;
    }

    ComPtr<IDXGIAdapter> adapter;
    hr = dxgi_device->GetAdapter(&adapter);
    if (FAILED(hr)) {
        return false;
    }

    ComPtr<IDXGIOutput> output;
    hr = adapter->EnumOutputs(0, &output);
    if (FAILED(hr)) {
        return false;
    }

    DXGI_OUTPUT_DESC desc;
    hr = output->GetDesc(&desc);
    if (FAILED(hr)) {
        return false;
    }

    RECT desktop_rect = desc.DesktopCoordinates;
    width_ = desktop_rect.right - desktop_rect.left;
    height_ = desktop_rect.bottom - desktop_rect.top;

    // 首先尝试 DXGI 1.5+ 的 DuplicateOutput1 (支持多并发捕获)
    ComPtr<IDXGIOutput5> output5;
    hr = output.As(&output5);
    ComPtr<IDXGIOutput1> output1;  // Declare here for goto to work
    ComPtr<IDXGIOutputDuplication> temp_duplication;

    if (SUCCEEDED(hr)) {
        std::cout << "[DXGI] IDXGIOutput5 available, trying DuplicateOutput1..." << std::endl;

        // DuplicateOutput1: 接受 5 参数
        // 关键: 必须传递有效的格式列表，不能使用 nullptr/0
        // HRESULT DuplicateOutput1(
        //     IUnknown *pDevice,
        //     UINT Flags,
        //     UINT SupportedFormatsCount,
        //     const DXGI_FORMAT *pSupportedFormats,
        //     IDXGIOutputDuplication **ppOutputDuplication
        // );

        // 指定支持的格式列表
        DXGI_FORMAT supported_formats[] = {
            DXGI_FORMAT_B8G8R8A8_UNORM,
        };

        hr = output5->DuplicateOutput1(
            d3d11_device_.Get(),
            0,                                      // Flags
            1,                                      // SupportedFormatsCount
            supported_formats,                      // pSupportedFormats
            &temp_duplication                       // ppOutputDuplication (输出参数)
        );

        if (SUCCEEDED(hr)) {
            std::cout << "[DXGI] DuplicateOutput1 succeeded - concurrent capture enabled!" << std::endl;
            duplication_ = temp_duplication.Detach();
            goto create_staging;
        }

        std::cerr << "[DXGI] DuplicateOutput1 failed: 0x" << std::hex << hr << std::dec << std::endl;
        if (hr == DXGI_ERROR_INVALID_CALL) {
            std::cerr << "[DXGI] Error: DXGI_ERROR_INVALID_CALL - driver may not support DuplicateOutput1" << std::endl;
        } else if (hr == E_ACCESSDENIED || hr == DXGI_ERROR_ACCESS_DENIED) {
            std::cerr << "[DXGI] Access denied even with DuplicateOutput1, trying legacy..." << std::endl;
        } else if (hr == ERROR_NOT_SUPPORTED) {
            std::cerr << "[DXGI] ERROR_NOT_SUPPORTED - DuplicateOutput1 not available" << std::endl;
        }
        std::cerr << "[DXGI] Falling back to legacy DuplicateOutput..." << std::endl;
    }

    // 回退到旧版 DuplicateOutput
    hr = output.As(&output1);
    if (FAILED(hr)) {
        return false;
    }

    hr = output1->DuplicateOutput(d3d11_device_.Get(), &duplication_);
    if (FAILED(hr)) {
        std::cerr << "[DXGI] Legacy DuplicateOutput failed: 0x" << std::hex << hr << std::dec << std::endl;
        if (hr == E_ACCESSDENIED || hr == DXGI_ERROR_ACCESS_DENIED) {
            std::cerr << "[DXGI] DXGI_ERROR_ACCESS_DENIED - Another app may be using Desktop Duplication" << std::endl;
            std::cerr << "[DXGI] Common causes:" << std::endl;
            std::cerr << "[DXGI]   - Windows Game Bar (Win+G)" << std::endl;
            std::cerr << "[DXGI]   - NVIDIA ShadowPlay / GeForce Experience" << std::endl;
            std::cerr << "[DXGI]   - Other screen recording software" << std::endl;
            std::cerr << "[DXGI]   - Remote desktop session" << std::endl;
        } else if (hr == ERROR_NOT_SUPPORTED || hr == E_INVALIDARG) {
            std::cerr << "[DXGI] Desktop Duplication not supported on this system" << std::endl;
        }
        return false;
    }

    std::cout << "[DXGI] Legacy DuplicateOutput succeeded" << std::endl;

create_staging:

    // 创建 staging texture
    D3D11_TEXTURE2D_DESC staging_desc = {};
    staging_desc.Width = width_;
    staging_desc.Height = height_;
    staging_desc.MipLevels = 1;
    staging_desc.ArraySize = 1;
    staging_desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    staging_desc.SampleDesc.Count = 1;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;

    hr = d3d11_device_->CreateTexture2D(&staging_desc, nullptr, &staging_texture_);
    if (FAILED(hr)) {
        return false;
    }

    return true;
}

bool D3D12HybridCapturer::CreateSharedResources() {
    // 创建 D3D11 共享纹理
    D3D11_TEXTURE2D_DESC shared_desc = {};
    shared_desc.Width = width_;
    shared_desc.Height = height_;
    shared_desc.MipLevels = 1;
    shared_desc.ArraySize = 1;
    shared_desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    shared_desc.SampleDesc.Count = 1;
    shared_desc.Usage = D3D11_USAGE_DEFAULT;
    shared_desc.MiscFlags = D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE;

    HRESULT hr = d3d11_device_->CreateTexture2D(&shared_desc, nullptr, &shared_texture_d3d11_);
    if (FAILED(hr)) {
        std::cerr << "Create shared D3D11 texture failed: " << std::hex << hr << std::endl;
        return false;
    }

    // 获取共享句柄
    ComPtr<IDXGIResource1> dxgi_resource;
    hr = shared_texture_d3d11_.As(&dxgi_resource);
    if (FAILED(hr)) {
        return false;
    }

    hr = dxgi_resource->CreateSharedHandle(
        nullptr,
        GENERIC_ALL,
        nullptr,
        &shared_handle_
    );
    if (FAILED(hr)) {
        std::cerr << "CreateSharedHandle failed: " << std::hex << hr << std::endl;
        return false;
    }

    // D3D12 打开共享资源
    hr = d3d12_device_->OpenSharedHandle(shared_handle_, __uuidof(shared_texture_d3d12_), &shared_texture_d3d12_);
    if (FAILED(hr)) {
        std::cerr << "OpenSharedHandle failed: " << std::hex << hr << std::endl;
        return false;
    }

    return true;
}

bool D3D12HybridCapturer::AcquireNextFrame() {
    if (!duplication_) {
        return false;
    }

    ComPtr<IDXGIResource> resource;
    DXGI_OUTDUPL_FRAME_INFO frame_info;

    HRESULT hr = duplication_->AcquireNextFrame(0, &frame_info, &resource);
    if (hr == DXGI_ERROR_WAIT_TIMEOUT) {
        return false;
    }

    if (FAILED(hr)) {
        if (hr == DXGI_ERROR_ACCESS_LOST) {
            duplication_->ReleaseFrame();
            duplication_.Reset();
            // TODO: 重新初始化
        }
        return false;
    }

    frame_info_ = frame_info;
    resource_ = resource;

    // 获取纹理
    ComPtr<ID3D11Texture2D> texture;
    hr = resource.As(&texture);
    if (FAILED(hr)) {
        duplication_->ReleaseFrame();
        return false;
    }

    captured_texture_d3d11_ = texture;

    // 如果启用 D3D12，复制到共享资源
    if (d3d12_enabled_ && shared_texture_d3d11_) {
        d3d11_context_->CopyResource(shared_texture_d3d11_.Get(), captured_texture_d3d11_.Get());
        d3d11_context_->Flush();
    }

    return true;
}

void D3D12HybridCapturer::ReleaseFrame() {
    if (duplication_) {
        duplication_->ReleaseFrame();
    }
    resource_.Reset();
    captured_texture_d3d11_.Reset();
}

bool D3D12HybridCapturer::Capture(D3D12HybridFrame* frame_info) {
    if (!initialized_) {
        return false;
    }

    // 释放之前的帧 (如果有)
    ReleaseFrame();

    if (!AcquireNextFrame()) {
        return false;  // 暂无新帧
    }

    if (frame_info) {
        frame_info->width = width_;
        frame_info->height = height_;
        frame_info->stride = width_ * 4;
        frame_info->format = DXGI_FORMAT_B8G8R8A8_UNORM;
        frame_info->timestamp = frame_info_.LastPresentTime.QuadPart;
        frame_info->d3d11_resource = captured_texture_d3d11_.Get();
        frame_info->d3d12_resource = d3d12_enabled_ ? shared_texture_d3d12_.Get() : nullptr;
    }

    return true;
}

bool D3D12HybridCapturer::CopyToCPU(unsigned char* buffer, int buffer_size) {
    if (!captured_texture_d3d11_ || !staging_texture_) {
        return false;
    }

    // 复制到 staging
    d3d11_context_->CopyResource(staging_texture_.Get(), captured_texture_d3d11_.Get());

    // 映射到 CPU
    D3D11_MAPPED_SUBRESOURCE mapped;
    HRESULT hr = d3d11_context_->Map(staging_texture_.Get(), 0, D3D11_MAP_READ, 0, &mapped);
    if (FAILED(hr)) {
        return false;
    }

    // 复制数据
    int row_size = width_ * 4;
    int required_size = row_size * height_;

    if (buffer_size >= required_size) {
        unsigned char* src = static_cast<unsigned char*>(mapped.pData);
        unsigned char* dst = buffer;

        for (int y = 0; y < height_; y++) {
            memcpy(dst, src, row_size);
            dst += row_size;
            src += mapped.RowPitch;
        }
    }

    d3d11_context_->Unmap(staging_texture_.Get(), 0);

    // 注意: 不在这里释放帧，由 Capture() 负责管理

    return buffer_size >= required_size;
}

void D3D12HybridCapturer::Release() {
    duplication_.Reset();
    staging_texture_.Reset();
    captured_texture_d3d11_.Reset();
    shared_texture_d3d11_.Reset();
    shared_texture_d3d12_.Reset();

    if (shared_handle_) {
        CloseHandle(shared_handle_);
        shared_handle_ = nullptr;
    }

    d3d12_list_.Reset();
    d3d12_allocator_.Reset();
    d3d12_queue_.Reset();
    d3d12_device_.Reset();

    d3d11_context_.Reset();
    d3d11_device_.Reset();

    initialized_ = false;
}
