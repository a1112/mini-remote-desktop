/**
 * Windows.Graphics.Capture (C++/WinRT) 实现
 *
 * 使用 C++/WinRT 2.0 实现
 * 支持 Windows 10 1803+ (Build 17134)
 */

#include "wgc_winrt.h"

// C++/WinRT headers
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Storage.Streams.h>
#include <Windows.Graphics.Capture.Interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>

// Standard headers
#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>
#include <iostream>
#include <vector>
#include <algorithm>
#include <mutex>
#include <dwmapi.h>

#pragma comment(lib, "dwmapi.lib")

// 定义导出
#define WGC_WINRT_EXPORTS

using namespace winrt;
using namespace Windows::Foundation;
using namespace Windows::Graphics::DirectX::Direct3D11;
using namespace Windows::Graphics::Capture;
using namespace Windows::Storage::Streams;

using Microsoft::WRL::ComPtr;

namespace {

// ============================================================================
// 辅助函数
// ============================================================================

// 从 D3D11 设备创建 IDirect3DDevice
ComPtr<IDirect3DDevice> CreateDirect3DDevice(ID3D11Device* d3d11_device) {
    ComPtr<IDXGIDevice> dxgi_device;
    HRESULT hr = d3d11_device->QueryInterface(&dxgi_device);
    if (FAILED(hr)) {
        return nullptr;
    }

    ComPtr<IInspectable> d3d11_device_inspectable;
    hr = CreateDirect3D11DeviceFromDXGIDevice(
        dxgi_device.Get(),
        &d3d11_device_inspectable
    );
    if (FAILED(hr)) {
        return nullptr;
    }

    ComPtr<IDirect3DDevice> d3d_device;
    hr = d3d11_device_inspectable->QueryInterface(&d3d_device);
    if (FAILED(hr)) {
        return nullptr;
    }

    return d3d_device;
}

// 从 IDirect3DSurface 获取 ID3D11Texture2D
ComPtr<ID3D11Texture2D> GetD3D11TextureFromSurface(IDirect3DSurface* surface) {
    ComPtr<IInspectable> surface_inspectable;
    HRESULT hr = surface->QueryInterface(&surface_inspectable);
    if (FAILED(hr)) {
        return nullptr;
    }

    ComPtr<IDirect3DDisplaySource> display_source;
    hr = surface_inspectable->QueryInterface(&display_source);
    if (FAILED(hr)) {
        return nullptr;
    }

    // 获取底层 D3D11 资源
    ComPtr<IDirect3DSurfaceInterop> surface_interop;
    hr = surface->QueryInterface(&surface_interop);
    if (FAILED(hr)) {
        return nullptr;
    }

    HANDLE handle = nullptr;
    hr = surface_interop->GetResource(&handle);
    if (FAILED(hr)) {
        return nullptr;
    }

    // 这里需要转换为 ID3D11Texture2D
    // 实际实现会更复杂，这里简化处理
    return nullptr;
}

// 创建 IDirect3DDevice
ComPtr<IDirect3DDevice> CreateDirect3DDevice(ID3D11Device* d3d11_device) {
    ComPtr<IDXGIDevice> dxgi_device;
    HRESULT hr = d3d11_device->QueryInterface(&dxgi_device);
    if (FAILED(hr)) {
        return nullptr;
    }

    // 使用 Windows.Graphics.DirectX.Direct3D11.Interop
    auto access = GetDXGIInterfaceFromObject(d3d11_device);

    ComPtr<IInspectable> device_inspectable;
    hr = CreateDirect3D11DeviceFromDXGIDevice(
        access.get(),
        &device_inspectable
    );

    if (FAILED(hr)) {
        return nullptr;
    }

    ComPtr<IDirect3DDevice> d3d_device;
    hr = device_inspectable->QueryInterface(__uuidof(IDirect3DDevice), &d3d_device);
    if (FAILED(hr)) {
        return nullptr;
    }

    return d3d_device;
}

}  // namespace

// ============================================================================
// WGC WinRT 捕获会话实现
// ============================================================================

class WgcWinRTSession {
public:
    WgcWinRTSession() : frame_id_(0), running_(false) {}

    bool Initialize(WgcWinRTType type, void* target) {
        // 创建 D3D11 设备
        D3D_FEATURE_LEVEL levels[] = {
            D3D_FEATURE_LEVEL_11_1,
            D3D_FEATURE_LEVEL_11_0,
        };

        HRESULT hr = D3D11CreateDevice(
            nullptr,
            D3D_DRIVER_TYPE_HARDWARE,
            nullptr,
            0,
            levels,
            ARRAYSIZE(levels),
            D3D11_SDK_VERSION,
            &d3d11_device_,
            &feature_level_,
            &d3d11_context_
        );

        if (FAILED(hr)) {
            std::cerr << "[WGC-WinRT] D3D11CreateDevice failed: 0x" << std::hex << hr << std::endl;
            return false;
        }

        std::cout << "[WGC-WinRT] D3D11 device created (FL: 0x" << std::hex << feature_level_ << ")" << std::endl;

        // 初始化 C++/WinRT
        init_apartment();

        // 创建 IDirect3DDevice
        direct3d_device_ = CreateDirect3DDevice(d3d11_device_.Get());
        if (!direct3d_device_) {
            std::cerr << "[WGC-WinRT] Failed to create Direct3D device" << std::endl;
            return false;
        }

        // 根据类型创建捕获项
        if (type == WGC_WINRT_MONITOR) {
            if (!InitializeMonitorCapture((int)(size_t)target)) {
                return false;
            }
        } else if (type == WGC_WINRT_WINDOW) {
            if (!InitializeWindowCapture((HWND)target)) {
                return false;
            }
        }

        return true;
    }

    bool InitializeMonitorCapture(int monitor_index) {
        // 获取监视器
        ComPtr<IDXGIOutput> output;
        HRESULT hr = GetDXGIOutput(monitor_index, &output);
        if (FAILED(hr)) {
            std::cerr << "[WGC-WinRT] Failed to get DXGI output " << monitor_index << std::endl;
            return false;
        }

        DXGI_OUTPUT_DESC desc;
        hr = output->GetDesc(&desc);
        if (FAILED(hr)) {
            return false;
        }

        width_ = desc.DesktopCoordinates.right - desc.DesktopCoordinates.left;
        height_ = desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top;

        // 使用 CaptureInterop 创建 GraphicsCaptureItem
        auto interop = capture_interop();

        try {
            capture_item_ = interop.CreateForMonitor(
                desc.Monitor,
                guid_of<IDirect3DDisplaySource>()
            );
        } catch (hresult_error const& e) {
            std::cerr << "[WGC-WinRT] CreateForMonitor failed: "
                      << winrt::to_string(e.message()) << std::endl;
            return false;
        }

        return SetupFramePool();
    }

    bool InitializeWindowCapture(HWND hwnd) {
        RECT rect;
        GetWindowRect(hwnd, &rect);
        width_ = rect.right - rect.left;
        height_ = rect.bottom - rect.top;

        // 使用 CaptureInterop 创建 GraphicsCaptureItem
        auto interop = capture_interop();

        try {
            capture_item_ = interop.CreateForWindow(
                hwnd,
                guid_of<IDirect3DDisplaySource>()
            );
        } catch (hresult_error const& e) {
            std::cerr << "[WGC-WinRT] CreateForWindow failed: "
                      << winrt::to_string(e.message()) << std::endl;
            return false;
        }

        return SetupFramePool();
    }

    bool SetupFramePool() {
        if (!capture_item_ || !direct3d_device_) {
            return false;
        }

        try {
            // 创建帧池
            frame_pool_ = Direct3D11CaptureFramePool::Create(
                direct3d_device_.as<IDirect3DDevice>(),
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,  // 缓冲 2 帧
                capture_item_.Size()
            );

            if (!frame_pool_) {
                std::cerr << "[WGC-WinRT] Failed to create frame pool" << std::endl;
                return false;
            }

            // 创建捕获会话
            session_ = frame_pool_.CreateCaptureSession(capture_item_);

            if (!session_) {
                std::cerr << "[WGC-WinRT] Failed to create capture session" << std::endl;
                return false;
            }

            // 设置是否捕获光标
            session_.IsCursorCaptureEnabled(false);

            // 注册帧到达事件
            frame_arrived_token_ = frame_pool_.FrameArrived(
                {this, &WgcWinRTSession::OnFrameArrived}
            );

            std::cout << "[WGC-WinRT] Capture session created: "
                      << width_ << "x" << height_ << std::endl;

        } catch (hresult_error const& e) {
            std::cerr << "[WGC-WinRT] Setup failed: "
                      << winrt::to_string(e.message()) << std::endl;
            return false;
        }

        return true;
    }

    bool Start() {
        if (!session_) {
            return false;
        }

        try {
            session_.StartCapture();
            running_ = true;
            std::cout << "[WGC-WinRT] Capture started" << std::endl;
            return true;
        } catch (hresult_error const& e) {
            std::cerr << "[WGC-WinRT] StartCapture failed: "
                      << winrt::to_string(e.message()) << std::endl;
            return false;
        }
    }

    void Stop() {
        if (session_) {
            try {
                session_.Close();
            } catch (...) {}
            session_ = nullptr;
        }

        if (frame_pool_) {
            try {
                frame_pool_.Close();
            } catch (...) {}
            frame_pool_ = nullptr;
        }

        capture_item_ = nullptr;
        direct3d_device_ = nullptr;
        running_ = false;
    }

    bool GetFrame(WgcWinRTFrame* frame) {
        std::lock_guard<std::mutex> lock(mutex_);

        if (!latest_surface_) {
            return false;
        }

        // 从 IDirect3DSurface 获取 D3D11 纹理
        // 注意: 这里需要使用 Interop API

        if (frame) {
            frame->width = width_;
            frame->height = height_;
            frame->d3d11_texture = nullptr;  // TODO: 从 surface 获取
            frame->timestamp = latest_timestamp_;
            frame->frame_id = frame_id_;
        }

        return true;
    }

    ID3D11Device* GetDevice() const { return d3d11_device_.Get(); }

private:
    HRESULT GetDXGIOutput(int index, IDXGIOutput** output) {
        ComPtr<IDXGIDevice> dxgi_device;
        HRESULT hr = d3d11_device_->QueryInterface(&dxgi_device);
        if (FAILED(hr)) {
            return hr;
        }

        ComPtr<IDXGIAdapter> adapter;
        hr = dxgi_device->GetAdapter(&adapter);
        if (FAILED(hr)) {
            return hr;
        }

        return adapter->EnumOutputs(index, output);
    }

    fire_and_forget OnFrameArrived(
        Direct3D11CaptureFramePool const& sender,
        IInspectable const& args)
    {
        auto frame = sender.TryGetNextFrame();
        if (!frame) {
            return;
        }

        std::lock_guard<std::mutex> lock(mutex_);

        latest_surface_ = frame.Surface();
        latest_timestamp_ = frame.SystemRelativeTime().count();
        frame_id_++;

        // 通知有新帧
        if (frame_event_) {
            SetEvent(frame_event_);
        }
    }

    ComPtr<ID3D11Device> d3d11_device_;
    ComPtr<ID3D11DeviceContext> d3d11_context_;
    D3D_FEATURE_LEVEL feature_level_;

    ComPtr<IDirect3DDevice> direct3d_device_;
    GraphicsCaptureItem capture_item_{nullptr};
    Direct3D11CaptureFramePool frame_pool_{nullptr};
    GraphicsCaptureSession session_{nullptr};

    IDirect3DSurface latest_surface_{nullptr};
    unsigned long long latest_timestamp_;
    unsigned int frame_id_;

    int width_;
    int height_;
    bool running_;

    std::mutex mutex_;
    HANDLE frame_event_ = nullptr;
    event_token frame_arrived_token_;
};

// ============================================================================
// 全局状态
// ============================================================================

static bool g_initialized = false;
static winrt::init_apartment_state g_apartment_state = winrt::init_apartment_state::uninitialized;

// ============================================================================
// C 接口实现
// ============================================================================

extern "C" {

WGC_WINRT_API int wgc_winrt_init() {
    if (g_initialized) {
        return 1;
    }

    try {
        // 初始化 C++/WinRT apartment
        g_apartment_state = init_apartment();
        g_initialized = true;

        std::cout << "[WGC-WinRT] Initialized" << std::endl;
        return 1;
    } catch (...) {
        std::cerr << "[WGC-WinRT] Failed to initialize" << std::endl;
        return 0;
    }
}

WGC_WINRT_API void wgc_winrt_cleanup() {
    if (g_initialized) {
        uninit_apartment(g_apartment_state);
        g_initialized = false;
    }
}

WGC_WINRT_API int wgc_winrt_is_supported() {
    // Windows.Graphics.Capture 需要 Windows 10 1803+
    // 可以检查版本号
    OSVERSIONINFOEXW osvi = {};
    osvi.dwOSVersionInfoSize = sizeof(osvi);

    // 简化检查 - 假设 Windows 10+ 都支持
    return 1;
}

WGC_WINRT_API int wgc_winrt_enum_monitors(WgcWinRTMonitorInfo* monitors, int max_count) {
    // 使用标准 API 枚举监视器
    struct MonitorData {
        std::vector<WgcWinRTMonitorInfo>* list;
        int index;
    };

    std::vector<WgcWinRTMonitorInfo> monitor_list;
    MonitorData data = {&monitor_list, 0};

    auto callback = [](HMONITOR hmon, HDC hdc, LPRECT rect, LPARAM lParam) -> BOOL {
        MonitorData* data = reinterpret_cast<MonitorData*>(lParam);

        MONITORINFOEXW mi = {};
        mi.cbSize = sizeof(mi);
        GetMonitorInfoW(hmon, &mi);

        WgcWinRTMonitorInfo info = {};
        info.index = data->index++;
        info.hmon = hmon;
        info.rect = mi.rcMonitor;
        info.width = mi.rcMonitor.right - mi.rcMonitor.left;
        info.height = mi.rcMonitor.bottom - mi.rcMonitor.top;
        info.is_primary = (mi.dwFlags & MONITORINFOF_PRIMARY) != 0;

        WideCharToMultiByte(CP_UTF8, 0, mi.szDevice, -1,
            (char*)info.name, sizeof(info.name), nullptr, nullptr);

        data->list->push_back(info);
        return TRUE;
    };

    EnumDisplayMonitors(nullptr, nullptr, callback, reinterpret_cast<LPARAM>(&data));

    int count = min((int)monitor_list.size(), max_count);
    for (int i = 0; i < count; i++) {
        monitors[i] = monitor_list[i];
    }

    return (int)monitor_list.size();
}

WGC_WINRT_API int wgc_winrt_enum_windows(WgcWinRTWindowInfo* windows, int max_count) {
    struct WindowData {
        std::vector<WgcWinRTWindowInfo>* list;
    };

    std::vector<WgcWinRTWindowInfo> window_list;
    WindowData data = {&window_list};

    auto callback = [](HWND hwnd, LPARAM lParam) -> BOOL {
        WindowData* data = reinterpret_cast<WindowData*>(lParam);

        if (!IsWindowVisible(hwnd)) {
            return TRUE;
        }

        wchar_t title[256];
        if (GetWindowTextW(hwnd, title, 256) == 0) {
            return TRUE;
        }

        if (wcslen(title) == 0) {
            return TRUE;
        }

        WgcWinRTWindowInfo info = {};
        info.hwnd = hwnd;
        info.is_visible = !IsIconic(hwnd);
        GetWindowRect(hwnd, &info.rect);
        wcsncpy_s(info.title, title, _TRUNCATE);

        data->list->push_back(info);
        return TRUE;
    };

    EnumWindows(callback, reinterpret_cast<LPARAM>(&data));

    int count = min((int)window_list.size(), max_count);
    for (int i = 0; i < count; i++) {
        windows[i] = window_list[i];
    }

    return (int)window_list.size();
}

WGC_WINRT_API HWgcWinRT wgc_winrt_create_session(WgcWinRTType type, void* target) {
    WgcWinRTSession* session = new WgcWinRTSession();

    if (!session->Initialize(type, target)) {
        delete session;
        return nullptr;
    }

    return static_cast<HWgcWinRT>(session);
}

WGC_WINRT_API int wgc_winrt_start(HWgcWinRT session) {
    WgcWinRTSession* impl = static_cast<WgcWinRTSession*>(session);
    return impl && impl->Start() ? 1 : 0;
}

WGC_WINRT_API void wgc_winrt_stop(HWgcWinRT session) {
    WgcWinRTSession* impl = static_cast<WgcWinRTSession*>(session);
    if (impl) {
        impl->Stop();
    }
}

WGC_WINRT_API int wgc_winrt_get_frame(HWgcWinRT session, WgcWinRTFrame* frame) {
    WgcWinRTSession* impl = static_cast<WgcWinRTSession*>(session);
    return impl && impl->GetFrame(frame) ? 1 : 0;
}

WGC_WINRT_API void* wgc_winrt_get_d3d11_device(HWgcWinRT session) {
    WgcWinRTSession* impl = static_cast<WgcWinRTSession*>(session);
    return impl ? impl->GetDevice() : nullptr;
}

WGC_WINRT_API void wgc_winrt_free_session(HWgcWinRT session) {
    WgcWinRTSession* impl = static_cast<WgcWinRTSession*>(session);
    if (impl) {
        impl->Stop();
        delete impl;
    }
}

}  // extern "C"
