/**
 * Windows.Graphics.Capture (WGC) 核心实现
 *
 * 使用 C++/WinRT 实现 Windows.Graphics.Capture
 * 支持: 屏幕捕获、窗口捕获、GPU Direct
 */

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Storage.Streams.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <Windows.Graphics.Capture.Interop.h>

#include <d3d11.h>
#include <dwmapi.h>
#include <iostream>
#include <sstream>

#pragma comment(lib, "dwmapi.lib")

using namespace winrt;
using namespace Windows::Foundation;
using namespace Windows::Graphics::DirectX::Direct3D11;
using namespace Windows::Graphics::Capture;
using namespace Windows::Storage::Streams;

// ============================================================================
// 辅助函数
// ============================================================================

namespace {

// 将 HWND 转换为 GraphicsCaptureItem
capture_item CreateCaptureItemFromWindow(HWND hwnd) {
    // 使用 Windows.Graphics.Capture.Interop
    auto interop = capture_interop();
    return interop.CreateForWindow(hwnd, winrt::guid_of<IDirect3DDisplaySource>());
}

// 将 HMONITOR 转换为 GraphicsCaptureItem
capture_item CreateCaptureItemFromMonitor(HMONITOR hmon) {
    auto interop = capture_interop();
    return interop.CreateForMonitor(hmon, winrt::guid_of<IDirect3DDisplaySource>());
}

// 创建 IDirect3DDevice
com_ptr<IDirect3DDevice> CreateD3DDevice(ID3D11Device* d3d11_device) {
    com_ptr<IDirect3DDisplaySource> display_source;
    // 使用 Direct3D11Interop 创建 IDirect3DDevice
    auto d3d11_interop = GetDXGIInterfaceFromObject(d3d11_device);

    com_ptr<IDirect3DDevice> device;
    // 创建 Direct3D 设备包装器
    CreateDirect3D11DeviceFromDXGIDevice(
        reinterpret_cast<::IInspectable*>(d3d11_interop.get()),
        reinterpret_cast<::IInspectable**>(put_abi(device))
    );

    return device;
}

}  // namespace

// ============================================================================
// WGC 捕获类
// ============================================================================

class WgcCaptureSession {
public:
    WgcCaptureSession() : frame_id_(0) {}

    bool Initialize(HWND hwnd) {
        return InitializeWindow(hwnd);
    }

    bool Initialize(HMONITOR hmon) {
        return InitializeMonitor(hmon);
    }

    bool Start() {
        if (!frame_pool_ || !session_) {
            return false;
        }

        // 设置帧回调
        frame_pool_.FrameArrived({this, &WgcCaptureSession::OnFrameArrived});

        // 启动捕获会话
        session_.StartCapture();
        return true;
    }

    void Stop() {
        if (session_) {
            session_.Close();
            session_ = nullptr;
        }
        if (frame_pool_) {
            frame_pool_.Close();
            frame_pool_ = nullptr;
        }
        if (item_) {
            item_ = nullptr;
        }
    }

    IDirect3DSurface GetLatestFrame() {
        std::lock_guard<std::mutex> lock(mutex_);
        return latest_frame_;
    }

    unsigned int GetFrameId() const {
        return frame_id_;
    }

    bool HasFrame() const {
        return latest_frame_ != nullptr;
    }

private:
    bool InitializeWindow(HWND hwnd) {
        try {
            // 创建捕获项
            auto interop = capture_interop();
            item_ = interop.CreateForWindow(hwnd, guid_of<IDirect3DDisplaySource>());

            return InitializeCommon();
        } catch (hresult_error const& e) {
            std::cerr << "[WGC] Failed to create window capture item: " << winrt::to_string(e.message()) << std::endl;
            return false;
        }
    }

    bool InitializeMonitor(HMONITOR hmon) {
        try {
            // 创建捕获项
            auto interop = capture_interop();
            item_ = interop.CreateForMonitor(hmon, guid_of<IDirect3DDisplaySource>());

            return InitializeCommon();
        } catch (hresult_error const& e) {
            std::cerr << "[WGC] Failed to create monitor capture item: " << winrt::to_string(e.message()) << std::endl;
            return false;
        }
    }

    bool InitializeCommon() {
        if (!item_) {
            return false;
        }

        // 获取设备
        auto d3d_device = CreateD3DDevice(d3d11_device_.get());
        if (!d3d_device) {
            std::cerr << "[WGC] Failed to create Direct3D device" << std::endl;
            return false;
        }

        // 创建帧池
        frame_pool_ = Direct3D11CaptureFramePool::Create(
            d3d_device.as<IDirect3DDevice>(),
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,  // 缓冲帧数
            item_.Size()
        );

        if (!frame_pool_) {
            std::cerr << "[WGC] Failed to create frame pool" << std::endl;
            return false;
        }

        // 创建捕获会话
        session_ = frame_pool_.CreateCaptureSession(item_);

        if (!session_) {
            std::cerr << "[WGC] Failed to create capture session" << std::endl;
            return false;
        }

        // 设置是否捕获光标
        session_.IsCursorCaptureEnabled(false);

        width_ = item_.Size().Width;
        height_ = item_.Size().Height;

        std::cout << "[WGC] Initialized: " << width_ << "x" << height_ << std::endl;
        return true;
    }

    void OnFrameArrived(Direct3D11CaptureFramePool const& sender, winrt::Windows::Foundation::IInspectable const& args) {
        auto frame = sender.TryGetNextFrame();
        if (!frame) {
            return;
        }

        std::lock_guard<std::mutex> lock(mutex_);
        latest_frame_ = frame.Surface();
        frame_id_++;

        // 通知新帧到达
        if (frame_event_) {
            SetEvent(frame_event_);
        }
    }

    com_ptr<ID3D11Device> d3d11_device_;
    com_ptr<ID3D11DeviceContext> d3d11_context_;

    capture_item item_;
    Direct3D11CaptureFramePool frame_pool_{nullptr};
    GraphicsCaptureSession session_{nullptr};

    IDirect3DSurface latest_frame_{nullptr};
    std::mutex mutex_;

    unsigned int frame_id_;
    int width_;
    int height_;

    HANDLE frame_event_ = nullptr;
};

// ============================================================================
// C 接口实现
// ============================================================================

extern "C" {

// ... (之前的 C 接口函数)

}  // extern "C"
