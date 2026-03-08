/**
 * DXGI Desktop Duplication Capture DLL
 *
 * 高性能屏幕捕获库 - 使用 DirectX 11 Desktop Duplication API
 *
 * 编译: Visual Studio 2022
 * 架构: x64
 *
 * 用法 (Python ctypes):
 *   dll = ctypes.CDLL('dxgi_capture.dll')
 *   dll.init_capture(0)  # 显示器索引
 *   buffer = ctypes.create_string_buffer(1920*1080*4)
 *   dll.get_frame(buffer, 1920, 1080)
 */
#pragma once

#include <windows.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>

using Microsoft::WRL::ComPtr;

// 帧数据结构
struct FrameInfo {
    int width;
    int height;
    int stride;
    DWORD format;  // DXGI_FORMAT
    ULONGLONG timestamp;
};

// 捕获句柄
typedef void* HCaptcha;

#ifdef __cplusplus
extern "C" {
#endif

/**
 * 初始化捕获器
 *
 * @param monitor_index 显示器索引 (0 = 主显示器)
 * @return 捕获句柄，NULL 表示失败
 */
HCaptcha __declspec(dllexport) init_capture(int monitor_index);

/**
 * 捕获一帧到缓冲区
 *
 * @param handle 捕获句柄
 * @param buffer 输出缓冲区 (BGRA 格式)
 * @param buffer_size 缓冲区大小
 * @param info 输出帧信息 (可选)
 * @return 1 成功, 0 失败, -1 需要重试
 */
int __declspec(dllexport) capture_frame(
    HCaptcha handle,
    unsigned char* buffer,
    int buffer_size,
    FrameInfo* info
);

/**
 * 释放捕获器
 */
void __declspec(dllexport) free_capture(HCaptcha handle);

/**
 * 获取显示器数量
 */
int __declspec(dllexport) get_monitor_count();

/**
 * 获取显示器信息
 */
int __declspec(dllexport) get_monitor_info(
    int index,
    int* width,
    int* height,
    int* is_primary
);

#ifdef __cplusplus
}
#endif


// 内部实现类
class DXGICapturer {
public:
    DXGICapturer();
    ~DXGICapturer();

    bool Initialize(int monitor_index);
    bool CaptureToBuffer(unsigned char* buffer, int buffer_size, FrameInfo* info);
    void Release();

    int GetWidth() const { return width_; }
    int GetHeight() const { return height_; }
    bool IsInitialized() const { return initialized_; }

private:
    bool CreateD3DDevice();
    bool GetOutput(int monitor_index);
    bool CreateDesktopDupl();
    bool AcquireNextFrame();
    bool CopyFrameToBuffer(unsigned char* buffer, int buffer_size);
    void ReleaseFrame();

    int width_ = 0;
    int height_ = 0;
    bool initialized_ = false;

    ComPtr<ID3D11Device> device_;
    ComPtr<ID3D11DeviceContext> context_;
    ComPtr<IDXGIOutputDuplication> duplication_;
    ComPtr<ID3D11Texture2D> staging_texture_;

    DXGI_OUTDUPL_FRAME_INFO frame_info_ = {};
    ComPtr<IDXGIResource> resource_;
    ComPtr<ID3D11Texture2D> frame_texture_;
};
