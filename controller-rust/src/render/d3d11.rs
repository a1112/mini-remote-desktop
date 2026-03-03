use super::RendererConfig;
use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{error, info};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

/// DirectX 11 渲染器（简化版本，显示视频统计）
pub struct D3D11Renderer {
    window: HWND,
    width: u32,
    height: u32,
    frame_count: Arc<std::sync::atomic::AtomicU64>,
    video_frames_received: Arc<std::sync::atomic::AtomicU64>,
    running: Arc<AtomicBool>,
}

impl D3D11Renderer {
    pub fn new(config: RendererConfig) -> Result<Self> {
        let video_frames_received = Arc::new(std::sync::atomic::AtomicU64::new(0));
        Self::new_with_stats(config, video_frames_received)
    }

    pub fn new_with_stats(
        config: RendererConfig,
        video_frames_received: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<Self> {
        let window = Self::create_window(config.window_width, config.window_height)?;
        let frame_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));

        // 启动渲染循环
        let window_handle: isize = window.0 as isize;
        let frame_count_clone = frame_count.clone();
        let video_frames_clone = video_frames_received.clone();
        let running_clone = running.clone();
        thread::spawn(move || {
            Self::render_loop(
                HWND(window_handle as *mut _),
                frame_count_clone,
                video_frames_clone,
                running_clone,
            );
        });

        info!("Video statistics renderer initialized");

        Ok(Self {
            window,
            width: config.window_width,
            height: config.window_height,
            frame_count,
            video_frames_received,
            running,
        })
    }

    /// 更新视频帧统计
    pub fn update_video_stats(&self, frame: &super::super::webrtc::peer::VideoFrame) {
        self.video_frames_received
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn render_loop(
        window: HWND,
        frame_count: Arc<std::sync::atomic::AtomicU64>,
        video_frames_received: Arc<std::sync::atomic::AtomicU64>,
        running: Arc<AtomicBool>,
    ) {
        let mut render_frame = 0usize;
        let mut msg = MSG::default();

        while running.load(Ordering::Relaxed) {
            unsafe {
                // 处理窗口消息（非阻塞，检索所有消息）
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                if !running.load(Ordering::Relaxed) {
                    break;
                }

                // 触发重绘
                let _ = InvalidateRect(Some(window), None, false);

                // 获取设备上下文进行绘制
                let hdc = GetDC(Some(window));
                if !hdc.is_invalid() {
                    let rendered = frame_count.load(Ordering::Relaxed);
                    let video_received = video_frames_received.load(Ordering::Relaxed);

                    // 获取窗口客户区大小
                    let mut rect = RECT::default();
                    let _ = GetClientRect(window, &mut rect);

                    // 设置背景
                    let bg_color = COLORREF(0x00141400); // RGB(20, 20, 10)
                    let bg_brush = CreateSolidBrush(bg_color);
                    FillRect(hdc, &rect, bg_brush);
                    let _ = DeleteObject(HGDIOBJ(bg_brush.0));

                    // 设置文本颜色
                    let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF)); // 白色
                    let _ = SetBkMode(hdc, BACKGROUND_MODE(1)); // TRANSPARENT

                    // 绘制统计信息
                    let text = format!(
                        "Remote Desktop - Rust Controller\r\n\
                         Rendered: {} frames ({} fps)\r\n\
                         Video Received: {} frames\r\n\
                         Video Rate: {} fps\r\n\
                         Frame Size: ~{} bytes",
                        rendered,
                        if render_frame > 0 { rendered * 60 / render_frame as u64 } else { 0 },
                        video_received,
                        if render_frame > 0 { video_received * 60 / render_frame as u64 } else { 0 },
                        2000
                    );

                    let mut text_wide: Vec<u16> = text.encode_utf16().collect();
                    let _ = DrawTextW(
                        hdc,
                        &mut text_wide,
                        &mut rect,
                        DT_LEFT | DT_TOP | DT_NOPREFIX,
                    );

                    let _ = ReleaseDC(Some(window), hdc);

                    render_frame += 1;
                    frame_count.fetch_add(1, Ordering::Relaxed);

                    if render_frame % 60 == 0 {
                        info!("rendered {} frames, video received {} frames", rendered, video_received);
                    }
                }
            }

            thread::sleep(Duration::from_millis(16)); // ~60fps
        }
        info!("render loop ended");
    }

    // 窗口过程函数（静态方法）
    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(window, &mut ps);
                let _ = EndPaint(window, &ps);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }

    fn create_window(width: u32, height: u32) -> Result<HWND> {
        unsafe {
            let instance = GetModuleHandleW(None)?;

            // 注册窗口类
            let class_name = windows::core::w!("ControllerWindow");
            let wnd_class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(Self::window_proc),
                hInstance: instance.into(),
                lpszClassName: class_name,
                hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _), // 默认背景色
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hIcon: HICON::default(),
                lpszMenuName: PCWSTR::null(),
            };

            let atom = RegisterClassW(&wnd_class);
            if atom == 0 {
                let error = GetLastError();
                if error != ERROR_CLASS_ALREADY_EXISTS {
                    return Err(anyhow::anyhow!("failed to register window class: {:?}", error));
                }
            }

            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                windows::core::w!("Remote Desktop - Rust Controller"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                width as i32,
                height as i32,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .context("failed to create window")?;

            info!(hwnd = ?window, "Window created");

            Ok(window)
        }
    }

    pub fn window_handle(&self) -> HWND {
        self.window
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl Drop for D3D11Renderer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // 给渲染线程一些时间退出
        thread::sleep(Duration::from_millis(50));
        unsafe {
            if !self.window.is_invalid() {
                let _ = DestroyWindow(self.window);
            }
        }
    }
}
