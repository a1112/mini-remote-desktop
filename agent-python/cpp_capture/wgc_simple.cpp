/**
 * Windows Graphics Capture (WGC) 简化实现
 *
 * 使用 Desktop Duplication API 实现
 * 延迟: ~0-1ms
 * GPU Direct: 支持
 */

#include "wgc_simple.h"
#include <d3d11.h>
#include <dxgi1_2.h>
#include <dxgi1_5.h>
#include <wrl/client.h>
#include <iostream>
#include <vector>
#include <algorithm>
#include <string>
#include <dwmapi.h>

#pragma comment(lib, "dwmapi.lib")

using Microsoft::WRL::ComPtr;

// 定义导出
#define WGC_EXPORTS

// ============================================================================
// 窗口/监视器数据结构
// ============================================================================

struct WindowEntry {
    HWND hwnd;
    std::wstring title;
    RECT rect;
    bool visible;
};

struct MonitorEntry {
    HMONITOR hmon;
    std::wstring name;
    RECT rect;
    bool is_primary;
};

static std::vector<WindowEntry> g_windows;
static std::vector<MonitorEntry> g_monitors;

// ============================================================================
// 枚举回调
// ============================================================================

static BOOL CALLBACK EnumWindowsProc(HWND hwnd, LPARAM lParam) {
    if (!IsWindowVisible(hwnd))
        return TRUE;

    wchar_t title[256];
    if (GetWindowTextW(hwnd, title, 256) == 0)
        return TRUE;

    if (wcslen(title) == 0)
        return TRUE;

    RECT rect;
    GetWindowRect(hwnd, &rect);

    g_windows.push_back({hwnd, std::wstring(title), rect, !IsIconic(hwnd)});
    return TRUE;
}

static BOOL CALLBACK EnumMonitorsProc(HMONITOR hmon, HDC hdc, LPRECT rect, LPARAM lParam) {
    MONITORINFOEXW mi = {};
    mi.cbSize = sizeof(mi);
    GetMonitorInfoW(hmon, &mi);

    g_monitors.push_back({
        hmon,
        std::wstring(mi.szDevice),
        mi.rcMonitor,
        (mi.dwFlags & MONITORINFOF_PRIMARY) != 0
    });
    return TRUE;
}

// ============================================================================
// WGC 捕获会话实现类
// ============================================================================

class WgcCaptureSessionImpl {
public:
    WgcCaptureSessionImpl()
        : frame_id_(0)
        , width_(0)
        , height_(0)
        , running_(false)
        , frame_ready_event_(nullptr)
    {
    }

    ~WgcCaptureSessionImpl() {
        Stop();
    }

    bool CreateD3DDevice() {
        D3D_FEATURE_LEVEL levels[] = {
            D3D_FEATURE_LEVEL_11_1,
            D3D_FEATURE_LEVEL_11_0,
        };

        D3D_FEATURE_LEVEL selected;
        HRESULT hr = D3D11CreateDevice(
            nullptr,
            D3D_DRIVER_TYPE_HARDWARE,
            nullptr,
            0,
            levels,
            2,
            D3D11_SDK_VERSION,
            &d3d11_device_,
            &selected,
            &d3d11_context_
        );

        if (FAILED(hr)) {
            std::cerr << "[WGC] D3D11CreateDevice failed: 0x" << std::hex << hr << std::endl;
            return false;
        }

        std::cout << "[WGC] D3D11 device created (Feature Level: 0x" << std::hex << selected << ")" << std::endl;
        return true;
    }

    bool Initialize(WgcCaptureType type, void* target) {
        if (!CreateD3DDevice()) {
            return false;
        }

        // 设置到输入桌面 (Lanthing 的做法)
        SetThreadDesktop();

        // 初始化 Desktop Duplication
        ComPtr<IDXGIDevice> dxgi_device;
        HRESULT hr = d3d11_device_.As(&dxgi_device);
        if (FAILED(hr)) {
            return false;
        }

        hr = dxgi_device->GetAdapter(&dxgi_adapter_);
        if (FAILED(hr)) {
            return false;
        }

        // 获取输出
        int output_index = (type == WGC_TYPE_MONITOR) ? (int)(size_t)target : 0;

        hr = dxgi_adapter_->EnumOutputs(output_index, &dxgi_output_);
        if (FAILED(hr)) {
            std::cerr << "[WGC] EnumOutputs failed for index " << output_index << std::endl;
            return false;
        }

        DXGI_OUTPUT_DESC desc;
        hr = dxgi_output_->GetDesc(&desc);
        if (FAILED(hr)) {
            return false;
        }

        width_ = desc.DesktopCoordinates.right - desc.DesktopCoordinates.left;
        height_ = desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top;

        // 尝试使用 DXGI 1.5 DuplicateOutput1
        ComPtr<IDXGIOutput5> output5;
        hr = dxgi_output_.As(&output5);

        ComPtr<IDXGIOutput1> output1;
        ComPtr<IDXGIOutputDuplication> temp_dup;

        if (SUCCEEDED(hr)) {
            std::cout << "[WGC] Trying DXGI 1.5 DuplicateOutput1 (concurrent capture)" << std::endl;

            DXGI_FORMAT formats[] = { DXGI_FORMAT_B8G8R8A8_UNORM };
            hr = output5->DuplicateOutput1(
                d3d11_device_.Get(),
                0,
                1,
                formats,
                &temp_dup
            );

            if (SUCCEEDED(hr)) {
                std::cout << "[WGC] DXGI 1.5 DuplicateOutput1 succeeded! Concurrent capture enabled." << std::endl;
                duplication_ = temp_dup;
                goto setup_complete;
            }

            std::cerr << "[WGC] DXGI 1.5 DuplicateOutput1 failed: 0x" << std::hex << hr << std::endl;
            if (hr == DXGI_ERROR_INVALID_CALL) {
                std::cerr << "[WGC] Driver doesn't support concurrent capture, trying legacy..." << std::endl;
            }
        }

        // 回退到 DuplicateOutput
        hr = dxgi_output_.As(&output1);
        if (FAILED(hr)) {
            return false;
        }

        hr = output1->DuplicateOutput(d3d11_device_.Get(), &duplication_);
        if (FAILED(hr)) {
            if (hr == DXGI_ERROR_ACCESS_DENIED) {
                std::cerr << "[WGC] ACCESS_DENIED - Another app is using Desktop Duplication" << std::endl;
                std::cerr << "[WGC] Close: Game Bar / NVIDIA Share / Screen Recorders" << std::endl;
            } else {
                std::cerr << "[WGC] DuplicateOutput failed: 0x" << std::hex << hr << std::endl;
            }
            return false;
        }

        std::cout << "[WGC] Legacy DuplicateOutput succeeded (exclusive mode)" << std::endl;

setup_complete:
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
            std::cerr << "[WGC] Create staging texture failed" << std::endl;
            return false;
        }

        frame_ready_event_ = CreateEvent(nullptr, FALSE, FALSE, nullptr);

        std::cout << "[WGC] Session initialized: " << width_ << "x" << height_ << std::endl;
        return true;
    }

    bool Start() {
        running_ = true;
        return true;
    }

    void Stop() {
        running_ = false;
        duplication_.Reset();
        current_texture_.Reset();
        staging_texture_.Reset();

        if (frame_ready_event_) {
            CloseHandle(frame_ready_event_);
            frame_ready_event_ = nullptr;
        }
    }

    bool GetFrame(WgcFrame* frame) {
        if (!running_ || !duplication_) {
            return false;
        }

        ComPtr<IDXGIResource> resource;
        DXGI_OUTDUPL_FRAME_INFO frame_info;

        // 使用 0ms 超时 (立即返回，用于高性能场景)
        // 如果需要降低 CPU 使用率，可以改回 1ms
        HRESULT hr = duplication_->AcquireNextFrame(0, &frame_info, &resource);
        if (hr == DXGI_ERROR_WAIT_TIMEOUT) {
            return false;  // 暂无新帧
        }

        if (FAILED(hr)) {
            if (hr == DXGI_ERROR_ACCESS_LOST) {
                std::cerr << "[WGC] DXGI_ERROR_ACCESS_LOST, attempting recovery..." << std::endl;
                duplication_->ReleaseFrame();

                // 尝试重新初始化
                if (ReinitializeDuplication()) {
                    return false;  // 成功恢复，等待下一帧
                }
            }
            return false;
        }

        // 获取纹理
        ComPtr<ID3D11Texture2D> texture;
        hr = resource.As(&texture);
        duplication_->ReleaseFrame();

        if (FAILED(hr)) {
            return false;
        }

        current_texture_ = texture;
        frame_id_++;

        if (frame) {
            frame->width = width_;
            frame->height = height_;
            frame->d3d11_texture = current_texture_.Get();
            frame->timestamp = frame_info.LastPresentTime.QuadPart;
            frame->frame_id = frame_id_;
        }

        return true;
    }

    bool ReinitializeDuplication() {
        std::cout << "[WGC] Reinitializing Desktop Duplication..." << std::endl;

        // 先释放现有资源
        duplication_.Reset();
        current_texture_.Reset();

        // 设置到输入桌面 (Lanthing 的做法)
        SetThreadDesktop();

        // 短暂等待
        Sleep(100);

        // 重新获取 DXGI 输出和创建 duplication
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
        hr = adapter->EnumOutputs(0, &output);  // 使用主输出
        if (FAILED(hr)) {
            return false;
        }

        ComPtr<IDXGIOutput1> output1;
        hr = output.As(&output1);
        if (FAILED(hr)) {
            return false;
        }

        // 使用传统 DuplicateOutput
        hr = output1->DuplicateOutput(d3d11_device_.Get(), &duplication_);

        if (SUCCEEDED(hr)) {
            std::cout << "[WGC] Reinitialization succeeded!" << std::endl;
            return true;
        }

        std::cerr << "[WGC] Reinitialization failed: 0x" << std::hex << hr << std::endl;
        return false;
    }

    void SetThreadDesktop() {
        // 设置线程到输入桌面 (Lanthing 的做法)
        HDESK current_desktop = GetThreadDesktop(GetCurrentThreadId());
        if (current_desktop) {
            CloseDesktop(current_desktop);
        }

        HDESK input_desktop = OpenInputDesktop(0, FALSE, GENERIC_ALL);
        if (input_desktop) {
            // 显式调用 Windows API
            ::SetThreadDesktop(input_desktop);
            CloseDesktop(input_desktop);
        }
    }

    bool CopyToCPU(void* buffer, int size) {
        if (!current_texture_ || !staging_texture_) {
            return false;
        }

        d3d11_context_->CopyResource(staging_texture_.Get(), current_texture_.Get());

        D3D11_MAPPED_SUBRESOURCE mapped;
        HRESULT hr = d3d11_context_->Map(staging_texture_.Get(), 0, D3D11_MAP_READ, 0, &mapped);
        if (FAILED(hr)) {
            return false;
        }

        int row_size = width_ * 4;
        int required_size = row_size * height_;

        if (size >= required_size) {
            unsigned char* src = (unsigned char*)mapped.pData;
            unsigned char* dst = (unsigned char*)buffer;

            for (int y = 0; y < height_; y++) {
                memcpy(dst, src, row_size);
                dst += row_size;
                src += mapped.RowPitch;
            }
        }

        d3d11_context_->Unmap(staging_texture_.Get(), 0);
        return size >= required_size;
    }

    ID3D11Device* GetDevice() const { return d3d11_device_.Get(); }
    ID3D11DeviceContext* GetContext() const { return d3d11_context_.Get(); }

private:
    ComPtr<ID3D11Device> d3d11_device_;
    ComPtr<ID3D11DeviceContext> d3d11_context_;
    ComPtr<ID3D11Texture2D> current_texture_;
    ComPtr<ID3D11Texture2D> staging_texture_;
    ComPtr<IDXGIOutputDuplication> duplication_;
    ComPtr<IDXGIAdapter> dxgi_adapter_;      // 用于重新初始化
    ComPtr<IDXGIOutput> dxgi_output_;        // 用于重新初始化

    unsigned int frame_id_;
    int width_;
    int height_;
    bool running_;
    HANDLE frame_ready_event_;
};

// ============================================================================
// C 接口实现
// ============================================================================

extern "C" {

WGC_API int wgc_enum_monitors(WgcMonitorInfo* monitors, int max_count) {
    g_monitors.clear();
    EnumDisplayMonitors(nullptr, nullptr, EnumMonitorsProc, 0);

    int count = min((int)g_monitors.size(), max_count);
    for (int i = 0; i < count; i++) {
        monitors[i].hmon = g_monitors[i].hmon;
        monitors[i].rect = g_monitors[i].rect;
        monitors[i].is_primary = g_monitors[i].is_primary;
        wcsncpy_s(monitors[i].name, g_monitors[i].name.c_str(), _TRUNCATE);
    }

    return (int)g_monitors.size();
}

WGC_API int wgc_enum_windows(WgcWindowInfo* windows, int max_count) {
    g_windows.clear();
    EnumWindows(EnumWindowsProc, 0);

    // 按标题排序
    std::sort(g_windows.begin(), g_windows.end(),
        [](const WindowEntry& a, const WindowEntry& b) {
            return a.title < b.title;
        });

    int count = min((int)g_windows.size(), max_count);
    for (int i = 0; i < count; i++) {
        windows[i].hwnd = g_windows[i].hwnd;
        windows[i].rect = g_windows[i].rect;
        windows[i].is_visible = g_windows[i].visible;
        wcsncpy_s(windows[i].title, g_windows[i].title.c_str(), _TRUNCATE);
    }

    return (int)g_windows.size();
}

WGC_API HWgcSession wgc_create_session(WgcCaptureType type, void* target) {
    WgcCaptureSessionImpl* session = new WgcCaptureSessionImpl();

    if (!session->Initialize(type, target)) {
        delete session;
        return nullptr;
    }

    return static_cast<HWgcSession>(session);
}

WGC_API int wgc_start(HWgcSession session) {
    WgcCaptureSessionImpl* impl = static_cast<WgcCaptureSessionImpl*>(session);
    return impl && impl->Start() ? 1 : 0;
}

WGC_API void wgc_stop(HWgcSession session) {
    WgcCaptureSessionImpl* impl = static_cast<WgcCaptureSessionImpl*>(session);
    if (impl) {
        impl->Stop();
    }
}

WGC_API int wgc_get_frame(HWgcSession session, WgcFrame* frame) {
    WgcCaptureSessionImpl* impl = static_cast<WgcCaptureSessionImpl*>(session);
    return impl && impl->GetFrame(frame) ? 1 : 0;
}

WGC_API int wgc_copy_to_cpu(HWgcSession session, void* buffer, int size) {
    WgcCaptureSessionImpl* impl = static_cast<WgcCaptureSessionImpl*>(session);
    return impl && impl->CopyToCPU(buffer, size) ? 1 : 0;
}

WGC_API void* wgc_get_d3d11_device(HWgcSession session) {
    WgcCaptureSessionImpl* impl = static_cast<WgcCaptureSessionImpl*>(session);
    return impl ? impl->GetDevice() : nullptr;
}

WGC_API void* wgc_get_d3d11_context(HWgcSession session) {
    WgcCaptureSessionImpl* impl = static_cast<WgcCaptureSessionImpl*>(session);
    return impl ? impl->GetContext() : nullptr;
}

WGC_API void wgc_free_session(HWgcSession session) {
    WgcCaptureSessionImpl* impl = static_cast<WgcCaptureSessionImpl*>(session);
    if (impl) {
        impl->Stop();
        delete impl;
    }
}

}  // extern "C"
