/**
 * Windows.Graphics.Capture 简化实现
 *
 * 使用纯 COM Interop API，不需要 C++/WinRT
 * 基于 Windows.Graphics.Capture.Interop 接口
 */

#pragma once

#include <windows.h>
#include <d3d11.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void* HWgcInterop;

typedef enum {
    WGC_INTEROP_MONITOR = 0,
    WGC_INTEROP_WINDOW = 1,
} WgcInteropType;

typedef struct {
    int width;
    int height;
    void* d3d11_texture;
    unsigned long long timestamp;
    unsigned int frame_id;
} WgcInteropFrame;

#ifdef WGC_INTEROP_EXPORTS
#define WGC_INTEROP_API __declspec(dllexport)
#else
#define WGC_INTEROP_API __declspec(dllimport)
#endif

// 初始化 (检查 WGC 支持并初始化 COM)
WGC_INTEROP_API int wgc_interop_init();

// 枚举监视器
WGC_INTEROP_API int wgc_interop_enum_monitors(void* monitors, int max_count);

// 枚举窗口
WGC_INTEROP_API int wgc_interop_enum_windows(void* windows, int max_count);

// 创建捕获会话
WGC_INTEROP_API HWgcInterop wgc_interop_create_session(WgcInteropType type, void* target);

// 启动捕获
WGC_INTEROP_API int wgc_interop_start(HWgcInterop session);

// 停止捕获
WGC_INTEROP_API void wgc_interop_stop(HWgcInterop session);

// 获取帧
WGC_INTEROP_API int wgc_interop_get_frame(HWgcInterop session, WgcInteropFrame* frame);

// 获取 D3D11 设备
WGC_INTEROP_API void* wgc_interop_get_d3d11_device(HWgcInterop session);

// 释放会话
WGC_INTEROP_API void wgc_interop_free_session(HWgcInterop session);

#ifdef __cplusplus
}
#endif
