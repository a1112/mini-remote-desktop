/**
 * Windows.Graphics.Capture (C++/WinRT) API
 *
 * 真正的 Windows.Graphics.Capture 实现
 * 支持: 并发捕获、窗口捕获、屏幕捕获、GPU Direct
 *
 * 延迟: ~0-1ms
 * 并发: 完全支持 (可与 Game Bar 等共存)
 */

#pragma once

#include <windows.h>
#include <d3d11.h>

// ============================================================================
// C 接口 (Python ctypes)
// ============================================================================

#ifdef __cplusplus
extern "C" {
#endif

// 不透明句柄
typedef void* HWgcWinRT;

// 捕获类型
typedef enum {
    WGC_WINRT_MONITOR = 0,
    WGC_WINRT_WINDOW = 1,
} WgcWinRTType;

// 捕获帧信息
typedef struct {
    int width;
    int height;
    void* d3d11_texture;      // ID3D11Texture2D*
    unsigned long long timestamp;
    unsigned int frame_id;
} WgcWinRTFrame;

// 窗口信息
typedef struct {
    HWND hwnd;
    wchar_t title[256];
    int is_visible;
    RECT rect;
} WgcWinRTWindowInfo;

// 监视器信息
typedef struct {
    int index;
    HMONITOR hmon;
    wchar_t name[64];
    RECT rect;
    int width;
    int height;
    int is_primary;
} WgcWinRTMonitorInfo;

#ifdef WGC_WINRT_EXPORTS
#define WGC_WINRT_API __declspec(dllexport)
#else
#define WGC_WINRT_API __declspec(dllimport)
#endif

// 初始化库 (必须首先调用)
WGC_WINRT_API int wgc_winrt_init();

// 清理库
WGC_WINRT_API void wgc_winrt_cleanup();

// 枚举监视器
WGC_WINRT_API int wgc_winrt_enum_monitors(WgcWinRTMonitorInfo* monitors, int max_count);

// 枚举窗口
WGC_WINRT_API int wgc_winrt_enum_windows(WgcWinRTWindowInfo* windows, int max_count);

// 创建捕获会话
WGC_WINRT_API HWgcWinRT wgc_winrt_create_session(WgcWinRTType type, void* target);

// 启动捕获
WGC_WINRT_API int wgc_winrt_start(HWgcWinRT session);

// 停止捕获
WGC_WINRT_API void wgc_winrt_stop(HWgcWinRT session);

// 获取最新帧 (非阻塞)
WGC_WINRT_API int wgc_winrt_get_frame(HWgcWinRT session, WgcWinRTFrame* frame);

// 复制帧到 CPU
WGC_WINRT_API int wgc_winrt_copy_to_cpu(HWgcWinRT session, void* buffer, int size);

// 获取 D3D11 设备
WGC_WINRT_API void* wgc_winrt_get_d3d11_device(HWgcWinRT session);

// 释放会话
WGC_WINRT_API void wgc_winrt_free_session(HWgcWinRT session);

// 检查是否支持 WGC API
WGC_WINRT_API int wgc_winrt_is_supported();

#ifdef __cplusplus
}
#endif
