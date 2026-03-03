/**
 * Windows.Graphics.Capture (WGC) API 实现
 *
 * 使用 Windows.Graphics.Capture.Interop 实现
 * 需要 Windows 10 1803+ (Build 17134)
 */

#include "wgc_capture.h"
#include <iostream>
#include <vector>
#include <algorithm>
#include <dwmapi.h>

// Windows Runtime headers
#include <windows.foundation.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <Windows.Graphics.Capture.Interop.h>

#pragma comment(lib, "dwmapi.lib")

// 定义导出
#define WGC_CAPTURE_EXPORTS

// ============================================================================
// 全局窗口/监视器列表
// ============================================================================

struct WindowEntry {
    HWND hwnd;
    std::string title;
};

static std::vector<WindowEntry> g_windows;
static std::vector<HMONITOR> g_monitors;

// ============================================================================
// 辅助函数
// ============================================================================

// 枚举窗口回调
static BOOL CALLBACK EnumWindowsProc(HWND hwnd, LPARAM lParam) {
    if (!IsWindowVisible(hwnd))
        return TRUE;

    // 获取窗口标题
    char title[256];
    if (GetWindowTextA(hwnd, title, sizeof(title)) == 0)
        return TRUE;

    // 过滤掉没有标题的窗口
    if (strlen(title) == 0)
        return TRUE;

    // 检查窗口是否最小化
    if (IsIconic(hwnd))
        return TRUE;

    g_windows.push_back({hwnd, std::string(title)});
    return TRUE;
}

// 枚举监视器回调
static BOOL CALLBACK EnumMonitorsProc(HMONITOR hmon, HDC hdc, LPRECT rect, LPARAM lParam) {
    g_monitors.push_back(hmon);
    return TRUE;
}

// 刷新窗口列表
static void RefreshWindowList() {
    g_windows.clear();
    EnumWindows(EnumWindowsProc, 0);
}

// 刷新监视器列表
static void RefreshMonitorList() {
    g_monitors.clear();
    EnumDisplayMonitors(NULL, NULL, EnumMonitorsProc, 0);
}

// 获取监视器信息
static bool GetMonitorInfoByIndex(int index, WgcMonitorInfo* info) {
    RefreshMonitorList();

    if (index < 0 || index >= (int)g_monitors.size())
        return false;

    MONITORINFOEXW mi = {};
    mi.cbSize = sizeof(mi);
    if (!GetMonitorInfoW(g_monitors[index], &mi))
        return false;

    info->index = index;
    info->rect_left = mi.rcMonitor.left;
    info->rect_top = mi.rcMonitor.top;
    info->rect_right = mi.rcMonitor.right;
    info->rect_bottom = mi.rcMonitor.bottom;
    info->width = mi.rcMonitor.right - mi.rcMonitor.left;
    info->height = mi.rcMonitor.bottom - mi.rcMonitor.top;
    info->is_primary = (mi.dwFlags & MONITORINFOF_PRIMARY) != 0;

    // 转换监视器名称
    WideCharToMultiByte(CP_UTF8, 0, mi.szDevice, -1, info->name, sizeof(info->name), NULL, NULL);

    return true;
}

// ============================================================================
// WgcCapturer 实现
// ============================================================================

WgcCapturer::WgcCapturer()
    : capture_item_(nullptr)
    , frame_pool_(nullptr)
    , session_(nullptr)
    , initialized_(false)
    , width_(0)
    , height_(0)
    , frame_id_(0)
    , frame_event_(nullptr)
    , stop_event_(nullptr)
{
}

WgcCapturer::~WgcCapturer() {
    Release();
}

bool WgcCapturer::CreateD3D11Device() {
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
        std::cerr << "[WGC] D3D11CreateDevice failed: 0x" << std::hex << hr << std::endl;
        return false;
    }

    std::cout << "[WGC] D3D11 device created successfully" << std::endl;
    return true;
}

bool WgcCapturer::Initialize(WgcCaptureType type, long long id) {
    // 创建 D3D11 设备
    if (!CreateD3D11Device()) {
        return false;
    }

    // 根据类型初始化
    if (type == WGC_CAPTURE_MONITOR) {
        if (!InitializeMonitorCapture((int)id)) {
            std::cerr << "[WGC] Failed to initialize monitor capture" << std::endl;
            return false;
        }
    } else if (type == WGC_CAPTURE_WINDOW) {
        if (!InitializeWindowCapture((HWND)id)) {
            std::cerr << "[WGC] Failed to initialize window capture" << std::endl;
            return false;
        }
    } else {
        std::cerr << "[WGC] Invalid capture type" << std::endl;
        return false;
    }

    // 创建事件
    frame_event_ = CreateEvent(nullptr, FALSE, FALSE, nullptr);
    stop_event_ = CreateEvent(nullptr, TRUE, FALSE, nullptr);

    if (!frame_event_ || !stop_event_) {
        std::cerr << "[WGC] Failed to create events" << std::endl;
        return false;
    }

    initialized_ = true;
    return true;
}

bool WgcCapturer::InitializeMonitorCapture(int monitor_index) {
    // 使用 IDXGIOutput 获取监视器
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
    hr = adapter->EnumOutputs(monitor_index, &output);
    if (FAILED(hr)) {
        std::cerr << "[WGC] Failed to get DXGI output " << monitor_index << std::endl;
        return false;
    }

    DXGI_OUTPUT_DESC desc;
    hr = output->GetDesc(&desc);
    if (FAILED(hr)) {
        return false;
    }

    width_ = desc.DesktopCoordinates.right - desc.DesktopCoordinates.left;
    height_ = desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top;

    std::cout << "[WGC] Monitor " << monitor_index << ": " << width_ << "x" << height_ << std::endl;

    // TODO: 使用 Windows.Graphics.Capture.Interop 创建捕获会话
    // 这需要 C++/WinRT 或 C++/CX 支持

    // 简化版本：使用 Desktop Duplication 作为回退
    std::cout << "[WGC] Note: Using Desktop Duplication as fallback (WGC interop requires additional setup)" << std::endl;

    return false;  // 暂时返回 false，需要完整实现 WGC interop
}

bool WgcCapturer::InitializeWindowCapture(HWND hwnd) {
    if (!IsWindow(hwnd)) {
        std::cerr << "[WGC] Invalid window handle" << std::endl;
        return false;
    }

    RECT rect;
    GetWindowRect(hwnd, &rect);
    width_ = rect.right - rect.left;
    height_ = rect.bottom - rect.top;

    std::cout << "[WGC] Window " << hwnd << ": " << width_ << "x" << height_ << std::endl;

    // TODO: 使用 Windows.Graphics.Capture.Interop 创建窗口捕获
    // 这需要 GraphicsCaptureItem::CreateFromWindowId

    return false;  // 暂时返回 false，需要完整实现 WGC interop
}

bool WgcCapturer::Capture(WgcFrameInfo* frame_info) {
    if (!initialized_) {
        return false;
    }

    // TODO: 实现帧捕获逻辑
    return false;
}

bool WgcCapturer::CopyToCPU(unsigned char* buffer, int buffer_size) {
    if (!captured_texture_ || !staging_texture_) {
        return false;
    }

    // TODO: 实现纹理复制
    return false;
}

void WgcCapturer::ProcessFrame(ID3D11Texture2D* texture) {
    captured_texture_ = texture;
    frame_id_++;
}

bool WgcCapturer::CopyToStaging(ID3D11Texture2D* source) {
    D3D11_TEXTURE2D_DESC staging_desc = {};
    staging_desc.Width = width_;
    staging_desc.Height = height_;
    staging_desc.MipLevels = 1;
    staging_desc.ArraySize = 1;
    staging_desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    staging_desc.SampleDesc.Count = 1;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;

    HRESULT hr = d3d11_device_->CreateTexture2D(&staging_desc, nullptr, &staging_texture_);
    if (FAILED(hr)) {
        return false;
    }

    d3d11_context_->CopyResource(staging_texture_.Get(), source);
    return true;
}

void WgcCapturer::Release() {
    if (stop_event_) {
        SetEvent(stop_event_);
    }

    if (session_) {
        session_->Release();
        session_ = nullptr;
    }

    if (frame_pool_) {
        frame_pool_->Release();
        frame_pool_ = nullptr;
    }

    if (capture_item_) {
        capture_item_->Release();
        capture_item_ = nullptr;
    }

    if (frame_event_) {
        CloseHandle(frame_event_);
        frame_event_ = nullptr;
    }

    if (stop_event_) {
        CloseHandle(stop_event_);
        stop_event_ = nullptr;
    }

    staging_texture_.Reset();
    captured_texture_.Reset();
    d3d11_context_.Reset();
    d3d11_device_.Reset();

    initialized_ = false;
}

// ============================================================================
// DLL 导出函数实现
// ============================================================================

extern "C" {

int wgc_get_monitor_count() {
    RefreshMonitorList();
    return (int)g_monitors.size();
}

int wgc_get_monitor_info(int index, WgcMonitorInfo* info) {
    if (!info) return 0;
    return GetMonitorInfoByIndex(index, info) ? 1 : 0;
}

int wgc_get_window_count() {
    RefreshWindowList();
    return (int)g_windows.size();
}

int wgc_get_window_info(int index, WgcWindowInfo* info) {
    if (!info) return 0;

    RefreshWindowList();

    if (index < 0 || index >= (int)g_windows.size())
        return 0;

    const auto& entry = g_windows[index];
    info->hwnd = entry.hwnd;
    strncpy_s(info->title, entry.title.c_str(), sizeof(info->title) - 1);
    info->is_minimized = IsIconic(entry.hwnd);
    info->is_visible = IsWindowVisible(entry.hwnd);

    RECT rect;
    GetWindowRect(entry.hwnd, &rect);
    info->rect_left = rect.left;
    info->rect_top = rect.top;
    info->rect_right = rect.right;
    info->rect_bottom = rect.bottom;

    return 1;
}

void wgc_refresh_windows() {
    RefreshWindowList();
}

HWgcCapture wgc_create_capture(WgcCaptureType type, long long id) {
    WgcCapturer* capturer = new WgcCapturer();

    if (!capturer->Initialize(type, id)) {
        delete capturer;
        return nullptr;
    }

    return static_cast<HWgcCapture>(capturer);
}

int wgc_capture_frame(HWgcCapture handle, WgcFrameInfo* frame_info) {
    WgcCapturer* capturer = static_cast<WgcCapturer*>(handle);
    if (!capturer || !capturer->IsInitialized()) {
        return 0;
    }

    if (capturer->Capture(frame_info)) {
        return 1;
    }

    return -1;
}

int wgc_copy_frame_to_cpu(HWgcCapture handle, unsigned char* buffer, int buffer_size) {
    WgcCapturer* capturer = static_cast<WgcCapturer*>(handle);
    if (!capturer || !capturer->IsInitialized()) {
        return 0;
    }

    return capturer->CopyToCPU(buffer, buffer_size) ? 1 : 0;
}

void* wgc_get_d3d11_device(HWgcCapture handle) {
    WgcCapturer* capturer = static_cast<WgcCapturer*>(handle);
    return capturer ? capturer->GetD3D11Device() : nullptr;
}

void* wgc_get_d3d11_context(HWgcCapture handle) {
    WgcCapturer* capturer = static_cast<WgcCapturer*>(handle);
    return capturer ? capturer->GetD3D11Context() : nullptr;
}

void wgc_free_capture(HWgcCapture handle) {
    WgcCapturer* capturer = static_cast<WgcCapturer*>(handle);
    if (capturer) {
        capturer->Release();
        delete capturer;
    }
}

}  // extern "C"
