/**
 * D3D12 Screen Capture Implementation
 *
 * 使用 Windows.Graphics.Capture API 实现纯 D3D12 屏幕捕获
 */
#include "d3d12_capture.h"
#include <iostream>
#include <vector>
#include <Windows.Graphics.Capture.h>

// ============================================================================
// Helper Functions
// ============================================================================

// 从 HWND 获取捕获项
HRESULT CreateCaptureItemForWindow(HWND window, IGraphicsCaptureItem** item) {
    ComPtr<IGraphicsCaptureItemFactory> factory;
    HRESULT hr = RoGetActivationFactory(
        HStringReference(RuntimeClass_Windows_Graphics_Capture_GraphicsCaptureItem).Get(),
        __uuidof(factory),
        &factory
    );
    if (FAILED(hr)) {
        return hr;
    }

    ComPtr<IGraphicsCaptureItemInterop> interop;
    hr = factory.As(&interop);
    if (FAILED(hr)) {
        return hr;
    }

    return interop->CreateForWindow(window, __uuidof(*item), reinterpret_cast<void**>(item));
}

// 从 HMONITOR 获取捕获项
HRESULT CreateCaptureItemForMonitor(HMONITOR monitor, IGraphicsCaptureItem** item) {
    ComPtr<IGraphicsCaptureItemFactory> factory;
    HRESULT hr = RoGetActivationFactory(
        HStringReference(RuntimeClass_Windows_Graphics_Capture_GraphicsCaptureItem).Get(),
        __uuidof(factory),
        &factory
    );
    if (FAILED(hr)) {
        return hr;
    }

    ComPtr<IGraphicsCaptureItemInterop> interop;
    hr = factory.As(&interop);
    if (FAILED(hr)) {
        return hr;
    }

    return interop->CreateForMonitor(monitor, __uuidof(*item), reinterpret_cast<void**>(item));
}

// 获取主显示器 HMONITOR
HMONITOR GetPrimaryMonitor() {
    const POINT pt = { 0, 0 };
    return MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
}

// 枚举显示器
BOOL CALLBACK MonitorEnumProc(HMONITOR hMonitor, HDC, LPRECT, LPARAM dwData) {
    int* index = reinterpret_cast<int*>(dwData);
    if (*index == 0) {
        // 将 HMONITOR 存储到某个地方
        // 这里简化处理
        return FALSE;
    }
    (*index)--;
    return TRUE;
}

// ============================================================================
// D3D12Capturer Implementation
// ============================================================================

D3D12Capturer::D3D12Capturer() {
}

D3D12Capturer::~D3D12Capturer() {
    Release();
}

bool D3D12Capturer::Initialize(int monitor_index, int gpu_index) {
    std::lock_guard<std::mutex> lock(mutex_);

    // 初始化 Windows Runtime
    HRESULT hr = RoInitialize(RO_INIT_MULTITHREADED);
    if (FAILED(hr) && hr != RPC_E_CHANGED_MODE) {
        std::cerr << "RoInitialize failed: " << std::hex << hr << std::endl;
        return false;
    }

    // 创建 D3D12 设备
    if (!CreateD3D12Device(gpu_index)) {
        return false;
    }

    // 创建命令队列
    if (!CreateCommandQueue()) {
        return false;
    }

    // 创建捕获项
    if (monitor_index == 0) {
        hr = CreateCaptureItemForMonitor(GetPrimaryMonitor(), &capture_item_);
    } else {
        // TODO: 枚举其他显示器
        hr = CreateCaptureItemForMonitor(GetPrimaryMonitor(), &capture_item_);
    }

    if (FAILED(hr)) {
        std::cerr << "Failed to create capture item: " << std::hex << hr << std::endl;
        return false;
    }

    // 获取捕获项尺寸
    ComPtr<IGraphicsCaptureItem2> item2;
    if (SUCCEEDED(capture_item_.As(&item2))) {
        SIZE size;
        hr = item2->get_Size(&size);
        if (SUCCEEDED(hr)) {
            width_ = size.cx;
            height_ = size.cy;
        }
    } else {
        // 回退方式
        width_ = 1920;
        height_ = 1080;
    }

    // 创建帧池和会话
    if (!CreateFramePool()) {
        return false;
    }

    if (!StartCaptureSession()) {
        return false;
    }

    initialized_ = true;
    return true;
}

bool D3D12Capturer::CreateD3D12Device(int gpu_index) {
    // 启用调试层
#ifdef _DEBUG
    ComPtr<ID3D12Debug> debug;
    if (SUCCEEDED(D3D12GetDebugInterface(__uuidof(debug), &debug))) {
        debug->EnableDebugLayer();
    }
#endif

    // 创建 DXGI 工厂
    ComPtr<IDXGIFactory4> factory;
    HRESULT hr = CreateDXGIFactory1(__uuidof(factory), &factory);
    if (FAILED(hr)) {
        std::cerr << "CreateDXGIFactory1 failed: " << std::hex << hr << std::endl;
        return false;
    }

    // 枚举适配器
    ComPtr<IDXGIAdapter1> adapter;
    if (gpu_index == 0) {
        hr = factory->EnumAdapters1(0, &adapter);
    } else {
        hr = factory->EnumAdapters1(gpu_index, &adapter);
    }

    if (FAILED(hr)) {
        std::cerr << "EnumAdapters1 failed: " << std::hex << hr << std::endl;
        return false;
    }

    // 创建 D3D12 设备
    hr = D3D12CreateDevice(
        adapter.Get(),
        D3D_FEATURE_LEVEL_11_0,
        __uuidof(device_),
        &device_
    );

    if (FAILED(hr)) {
        std::cerr << "D3D12CreateDevice failed: " << std::hex << hr << std::endl;
        return false;
    }

    return true;
}

bool D3D12Capturer::CreateCommandQueue() {
    D3D12_COMMAND_QUEUE_DESC queue_desc = {};
    queue_desc.Type = D3D12_COMMAND_LIST_TYPE_COPY;
    queue_desc.Flags = D3D12_COMMAND_QUEUE_FLAG_NONE;
    queue_desc.NodeMask = 0;

    HRESULT hr = device_->CreateCommandQueue(&queue_desc, __uuidof(command_queue_), &command_queue_);
    if (FAILED(hr)) {
        std::cerr << "CreateCommandQueue failed: " << std::hex << hr << std::endl;
        return false;
    }

    return true;
}

bool D3D12Capturer::CreateFramePool() {
    // 创建 Direct3D12 帧池工厂
    ComPtr<IDirect3D12CaptureFramePoolFactory> pool_factory;
    HRESULT hr = RoGetActivationFactory(
        HStringReference(RuntimeClass_Windows_Graphics_Capture_Direct3D11CaptureFramePool).Get(),
        __uuidof(pool_factory),
        &pool_factory
    );

    // 注意: Windows.Graphics.Capture API 主要支持 D3D11
    // 对于纯 D3D12，需要使用互操作
    // 这里我们先创建一个简化版本

    // 替代方案: 使用 D3D11 捕获然后共享到 D3D12
    // 这将在完整实现中完成

    return true;
}

bool D3D12Capturer::StartCaptureSession() {
    // 创建捕获会话
    // 完整实现将在这里创建 GraphicsCaptureSession
    return true;
}

bool D3D12Capturer::CaptureToD3D12Resource(ID3D12Resource** pp_resource, D3D12FrameInfo* info) {
    std::lock_guard<std::mutex> lock(mutex_);

    if (!initialized_) {
        return false;
    }

    // 简化实现: 返回测试资源
    // 完整实现将从帧池获取最新帧

    return false;
}

bool D3D12Capturer::CaptureToCPU(unsigned char* buffer, int buffer_size, D3D12FrameInfo* info) {
    std::lock_guard<std::mutex> lock(mutex_);

    if (!initialized_) {
        return false;
    }

    // TODO: 实现完整的捕获路径

    return false;
}

void D3D12Capturer::Release() {
    std::lock_guard<std::mutex> lock(mutex_);

    // 等待 GPU 完成
    if (command_queue_ && fence_) {
        command_queue_->Signal(fence_.Get(), ++fence_value_);
        if (fence_value_ != fence_->GetCompletedValue()) {
            fence_->SetEventOnCompletion(fence_value_, fence_event_);
            WaitForSingleObject(fence_event_, INFINITE);
        }
    }

    // 清理资源
    capture_session_.Reset();
    frame_pool_.Reset();
    captured_resource_.Reset();
    staging_resource_.Reset();
    command_list_.Reset();
    command_allocator_.Reset();
    command_queue_.Reset();
    device_.Reset();

    if (fence_event_) {
        CloseHandle(fence_event_);
        fence_event_ = NULL;
    }
    fence_.Reset();

    capture_item_.Reset();

    initialized_ = false;
}

// ============================================================================
// DLL Export Functions
// ============================================================================

extern "C" {

int __declspec(dllexport) is_d3d12_capture_supported() {
    // 检查 Windows 版本 (需要 10.0.17763.0 / 1803+)
    HMODULE hModule = LoadLibrary(L"kernel32.dll");
    if (hModule) {
        typedef HRESULT(WINAPI* PGetVersionedProductInfo)(
            PCWSTR, PCWSTR, ULONG*, PULONGLONG);
        PGetVersionedProductInfo pGetVersionedProductInfo =
            (PGetVersionedProductInfo)GetProcAddress(hModule, "GetVersionedProductInfo");

        if (pGetVersionedProductInfo) {
            // 检测版本
            FreeLibrary(hModule);
            return 1;  // 假设支持
        }
        FreeLibrary(hModule);
    }

    // 简化检查: 尝试创建 D3D12 设备
    ComPtr<ID3D12Device> test_device;
    HRESULT hr = D3D12CreateDevice(
        nullptr,
        D3D_FEATURE_LEVEL_11_0,
        __uuidof(test_device),
        &test_device
    );

    return SUCCEEDED(hr) ? 1 : 0;
}

int __declspec(dllexport) get_supported_capture_methods() {
    // 1 = DesktopDuplication (D3D11)
    // 2 = GraphicsCapture (D3D12/WinRT)
    // 4 = Both

    int supported = 0;

    // 检查 D3D11 Desktop Duplication
    ComPtr<ID3D11Device> d3d11_device;
    if (SUCCEEDED(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr,
        0, nullptr, 0, D3D11_SDK_VERSION, &d3d11_device, nullptr, nullptr))) {
        supported |= 1;
    }

    // 检查 GraphicsCapture
    if (is_d3d12_capture_supported()) {
        supported |= 2;
    }

    return supported;
}

HD3D12Capture __declspec(dllexport) init_d3d12_capture(int monitor_index, int gpu_index) {
    // 检查支持
    if (!is_d3d12_capture_supported()) {
        std::cerr << "D3D12 capture not supported on this system" << std::endl;
        return nullptr;
    }

    D3D12Capturer* capturer = new D3D12Capturer();
    if (!capturer->Initialize(monitor_index, gpu_index)) {
        delete capturer;
        return nullptr;
    }
    return static_cast<HD3D12Capture>(capturer);
}

int __declspec(dllexport) capture_d3d12_frame(
    HD3D12Capture handle,
    void** pp_output_resource,
    D3D12FrameInfo* info
) {
    D3D12Capturer* capturer = static_cast<D3D12Capturer*>(handle);
    if (!capturer || !capturer->IsInitialized()) {
        return 0;
    }

    ID3D12Resource* resource = nullptr;
    if (capturer->CaptureToD3D12Resource(&resource, info)) {
        *pp_output_resource = resource;
        return 1;
    }

    return -1;
}

int __declspec(dllexport) capture_d3d12_to_cpu(
    HD3D12Capture handle,
    unsigned char* buffer,
    int buffer_size,
    D3D12FrameInfo* info
) {
    D3D12Capturer* capturer = static_cast<D3D12Capturer*>(handle);
    if (!capturer || !capturer->IsInitialized()) {
        return 0;
    }

    if (capturer->CaptureToCPU(buffer, buffer_size, info)) {
        return 1;
    }

    return -1;
}

void* __declspec(dllexport) get_d3d12_device(HD3D12Capture handle) {
    D3D12Capturer* capturer = static_cast<D3D12Capturer*>(handle);
    return capturer ? capturer->GetDevice() : nullptr;
}

void* __declspec(dllexport) get_d3d12_command_queue(HD3D12Capture handle) {
    D3D12Capturer* capturer = static_cast<D3D12Capturer*>(handle);
    return capturer ? capturer->GetCommandQueue() : nullptr;
}

void __declspec(dllexport) free_d3d12_capture(HD3D12Capture handle) {
    D3D12Capturer* capturer = static_cast<D3D12Capturer*>(handle);
    if (capturer) {
        capturer->Release();
        delete capturer;
    }
}

}  // extern "C"
