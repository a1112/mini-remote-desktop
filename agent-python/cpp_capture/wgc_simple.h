/**
 * Windows Graphics Capture (WGC) 简化实现
 *
 * 使用 Desktop Duplication API 作为实现
 * 提供 WGC 风格的接口：窗口枚举、监视器枚举、GPU Direct 输出
 */

#pragma once

#include <windows.h>
#include <d3d11.h>

// ============================================================================
// 不透明句柄和结构体
// ============================================================================

typedef void* HWgcSession;

typedef enum {
    WGC_TYPE_MONITOR = 0,
    WGC_TYPE_WINDOW = 1,
} WgcCaptureType;

typedef struct {
    int width;
    int height;
    void* d3d11_texture;      // ID3D11Texture2D*
    unsigned long long timestamp;
    unsigned int frame_id;
} WgcFrame;

typedef struct {
    HWND hwnd;
    wchar_t title[256];
    int is_visible;
    RECT rect;
} WgcWindowInfo;

typedef struct {
    HMONITOR hmon;
    wchar_t name[64];
    RECT rect;
    int is_primary;
} WgcMonitorInfo;

// ============================================================================
// DLL 导出
// ============================================================================

#ifdef __cplusplus
extern "C" {
#endif

#ifdef WGC_EXPORTS
#define WGC_API __declspec(dllexport)
#else
#define WGC_API __declspec(dllimport)
#endif

// 枚举监视器
WGC_API int wgc_enum_monitors(WgcMonitorInfo* monitors, int max_count);

// 枚举窗口
WGC_API int wgc_enum_windows(WgcWindowInfo* windows, int max_count);

// 创建捕获会话 (target = 监视器索引 或 HWND)
WGC_API HWgcSession wgc_create_session(WgcCaptureType type, void* target);

// 启动捕获
WGC_API int wgc_start(HWgcSession session);

// 停止捕获
WGC_API void wgc_stop(HWgcSession session);

// 获取最新帧
WGC_API int wgc_get_frame(HWgcSession session, WgcFrame* frame);

// 复制帧到 CPU
WGC_API int wgc_copy_to_cpu(HWgcSession session, void* buffer, int size);

// 获取 D3D11 设备
WGC_API void* wgc_get_d3d11_device(HWgcSession session);

// 获取 D3D11 上下文
WGC_API void* wgc_get_d3d11_context(HWgcSession session);

// 释放会话
WGC_API void wgc_free_session(HWgcSession session);

#ifdef __cplusplus
}
#endif
