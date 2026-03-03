use super::RendererConfig;
use crate::video::decoder::DecodedFrame;
use anyhow::{Context, Result};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use windows::core::PCSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCWSTR};

#[derive(Default)]
struct SharedFrame {
    latest: Option<DecodedFrame>,
    sequence: u64,
}

pub struct D3D11Renderer {
    window: HWND,
    frame_count: Arc<AtomicU64>,
    video_frames_received: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    shared_frame: Arc<Mutex<SharedFrame>>,
}

#[derive(Clone)]
pub struct D3D11FrameSink {
    video_frames_received: Arc<AtomicU64>,
    shared_frame: Arc<Mutex<SharedFrame>>,
}

impl D3D11FrameSink {
    pub fn submit(&self, frame: DecodedFrame) {
        self.video_frames_received.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut shared) = self.shared_frame.lock() {
            shared.sequence = shared.sequence.wrapping_add(1);
            shared.latest = Some(frame);
        }
    }
}

impl D3D11Renderer {
    pub fn new(config: RendererConfig) -> Result<Self> {
        let video_frames_received = Arc::new(AtomicU64::new(0));
        Self::new_with_stats(config, video_frames_received)
    }

    pub fn new_with_stats(
        config: RendererConfig,
        video_frames_received: Arc<AtomicU64>,
    ) -> Result<Self> {
        let window = Self::create_window(config.window_width, config.window_height)?;
        let frame_count = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let shared_frame = Arc::new(Mutex::new(SharedFrame::default()));

        let window_handle: isize = window.0 as isize;
        let frame_count_clone = frame_count.clone();
        let video_frames_clone = video_frames_received.clone();
        let running_clone = running.clone();
        let shared_frame_clone = shared_frame.clone();
        thread::spawn(move || {
            unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
            }
            if let Err(e) = Self::render_loop(
                HWND(window_handle as *mut _),
                frame_count_clone,
                video_frames_clone,
                running_clone,
                shared_frame_clone,
                config.vsync,
            ) {
                error!(error = %e, "render loop failed");
            }
        });

        info!("D3D11 video renderer initialized");
        Ok(Self {
            window,
            frame_count,
            video_frames_received,
            running,
            shared_frame,
        })
    }

    pub fn submit_decoded_frame(&self, frame: DecodedFrame) {
        self.frame_sink().submit(frame);
    }

    pub fn frame_sink(&self) -> D3D11FrameSink {
        D3D11FrameSink {
            video_frames_received: self.video_frames_received.clone(),
            shared_frame: self.shared_frame.clone(),
        }
    }

    pub fn update_video_stats(&self, _frame: &super::super::webrtc::peer::VideoFrame) {
        self.video_frames_received.fetch_add(1, Ordering::Relaxed);
    }

    fn render_loop(
        window: HWND,
        frame_count: Arc<AtomicU64>,
        video_frames_received: Arc<AtomicU64>,
        running: Arc<AtomicBool>,
        shared_frame: Arc<Mutex<SharedFrame>>,
        vsync: bool,
    ) -> Result<()> {
        let mut msg = MSG::default();
        let started_at = Instant::now();
        let mut last_frame_sequence = 0u64;
        let mut present_samples_ms: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(1024);
        let mut last_present_stats = Instant::now();

        let mut d3d = D3DContext::new(window, vsync)?;

        while running.load(Ordering::Relaxed) {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if !running.load(Ordering::Relaxed) {
                break;
            }

            let maybe_frame = {
                let mut guard = match shared_frame.lock() {
                    Ok(g) => g,
                    Err(_) => {
                        warn!("frame mutex poisoned");
                        break;
                    }
                };
                if guard.sequence == last_frame_sequence {
                    None
                } else {
                    last_frame_sequence = guard.sequence;
                    guard.latest.take()
                }
            };

            if let Some(frame) = maybe_frame {
                d3d.upload_nv12(&frame)?;
                d3d.draw_frame()?;
                if frame.capture_start_unix_us != 0 {
                    if let Ok(elapsed) =
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    {
                        let now_us = elapsed.as_micros().min(u64::MAX as u128) as u64;
                        if now_us >= frame.capture_start_unix_us {
                            let e2e_ms = (now_us - frame.capture_start_unix_us) as f64 / 1000.0;
                            if present_samples_ms.len() >= 1024 {
                                present_samples_ms.pop_front();
                            }
                            present_samples_ms.push_back(e2e_ms);
                        }
                    }
                }
                frame_count.fetch_add(1, Ordering::Relaxed);
            } else {
                unsafe {
                    let _ = MsgWaitForMultipleObjectsEx(None, 5, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                }
                continue;
            }

            if last_present_stats.elapsed() >= Duration::from_secs(2) && !present_samples_ms.is_empty() {
                let mut sorted: Vec<f64> = present_samples_ms.iter().copied().collect();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let idx = |p: f64| -> usize {
                    ((sorted.len() as f64) * p)
                        .floor()
                        .min((sorted.len().saturating_sub(1)) as f64) as usize
                };
                let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;
                info!(
                    capture_to_present_avg_ms = format!("{:.3}", avg),
                    capture_to_present_p50_ms = format!("{:.3}", sorted[idx(0.50)]),
                    capture_to_present_p95_ms = format!("{:.3}", sorted[idx(0.95)]),
                    capture_to_present_p99_ms = format!("{:.3}", sorted[idx(0.99)]),
                    samples = sorted.len(),
                    "[PRESENT-STATS]"
                );
                last_present_stats = Instant::now();
            }

            let rendered = frame_count.load(Ordering::Relaxed);
            if rendered % 240 == 0 && rendered > 0 {
                let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                let render_fps = (rendered as f64 / elapsed).round() as u64;
                let recv = video_frames_received.load(Ordering::Relaxed);
                let recv_fps = (recv as f64 / elapsed).round() as u64;
                info!(
                    rendered_frames = rendered,
                    rendered_fps = render_fps,
                    received_frames = recv,
                    received_fps = recv_fps,
                    "renderer progress"
                );
            }
        }

        info!("render loop ended");
        Ok(())
    }

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
            let class_name = windows::core::w!("ControllerWindowD3D11");
            let wnd_class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(Self::window_proc),
                hInstance: instance.into(),
                lpszClassName: class_name,
                hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
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
                    return Err(anyhow::anyhow!("register class failed: {:?}", error));
                }
            }

            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                windows::core::w!("Remote Desktop - D3D11"),
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
            Ok(window)
        }
    }

    pub fn window_handle(&self) -> HWND {
        self.window
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl Drop for D3D11Renderer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(50));
        unsafe {
            if !self.window.is_invalid() {
                let _ = DestroyWindow(self.window);
            }
        }
    }
}

struct D3DContext {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain,
    rtv: ID3D11RenderTargetView,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    y_tex: Option<ID3D11Texture2D>,
    uv_tex: Option<ID3D11Texture2D>,
    y_srv: Option<ID3D11ShaderResourceView>,
    uv_srv: Option<ID3D11ShaderResourceView>,
    frame_width: u32,
    frame_height: u32,
    vsync: bool,
}

impl D3DContext {
    fn new(window: HWND, vsync: bool) -> Result<Self> {
        unsafe {
            let mut swap_desc = DXGI_SWAP_CHAIN_DESC::default();
            swap_desc.BufferCount = 2;
            swap_desc.BufferDesc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
            swap_desc.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
            swap_desc.OutputWindow = window;
            swap_desc.SampleDesc.Count = 1;
            swap_desc.Windowed = TRUE;
            swap_desc.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;

            let feature_levels = [D3D_FEATURE_LEVEL_11_0];
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let mut swap_chain: Option<IDXGISwapChain> = None;
            let mut chosen_level = D3D_FEATURE_LEVEL_11_0;

            D3D11CreateDeviceAndSwapChain(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&swap_desc),
                Some(&mut swap_chain),
                Some(&mut device),
                Some(&mut chosen_level),
                Some(&mut context),
            )
            .context("D3D11CreateDeviceAndSwapChain failed")?;

            let device = device.context("missing D3D11 device")?;
            let context = context.context("missing D3D11 context")?;
            let swap_chain = swap_chain.context("missing swap chain")?;
            if let Ok(dxgi_device1) = device.cast::<IDXGIDevice1>() {
                let _ignored = unsafe { dxgi_device1.SetMaximumFrameLatency(1) };
            }

            let back_buffer: ID3D11Texture2D = swap_chain
                .GetBuffer(0)
                .context("swap chain GetBuffer failed")?;
            let mut rtv = None;
            device
                .CreateRenderTargetView(&back_buffer, None, Some(&mut rtv))
                .context("CreateRenderTargetView failed")?;
            let rtv = rtv.context("missing render target view")?;

            let vs_src = b"
struct VSOut {
    float4 pos : SV_POSITION;
    float2 uv  : TEXCOORD0;
};
VSOut main(uint vid : SV_VertexID) {
    float2 p[3];
    p[0] = float2(-1.0, -1.0);
    p[1] = float2(-1.0,  3.0);
    p[2] = float2( 3.0, -1.0);
    VSOut o;
    o.pos = float4(p[vid], 0.0, 1.0);
    o.uv = float2((p[vid].x + 1.0) * 0.5, 1.0 - (p[vid].y + 1.0) * 0.5);
    return o;
}";

            let ps_src = b"
Texture2D texY  : register(t0);
Texture2D texUV : register(t1);
SamplerState samp : register(s0);
float4 main(float4 pos : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    float y = texY.Sample(samp, uv).r;
    float2 uvv = texUV.Sample(samp, uv).rg;
    float u = uvv.x - 0.5;
    float v = uvv.y - 0.5;
    float r = y + 1.402 * v;
    float g = y - 0.344136 * u - 0.714136 * v;
    float b = y + 1.772 * u;
    return float4(saturate(r), saturate(g), saturate(b), 1.0);
}";

            let vs_blob = compile_hlsl(vs_src, b"main\0", b"vs_5_0\0")?;
            let ps_blob = compile_hlsl(ps_src, b"main\0", b"ps_5_0\0")?;

            let mut vs = None;
            let vs_bytes = std::slice::from_raw_parts(
                vs_blob.GetBufferPointer() as *const u8,
                vs_blob.GetBufferSize(),
            );
            device
                .CreateVertexShader(
                    vs_bytes,
                    None,
                    Some(&mut vs),
                )
                .context("CreateVertexShader failed")?;
            let vs = vs.context("missing vertex shader")?;
            let mut ps = None;
            let ps_bytes = std::slice::from_raw_parts(
                ps_blob.GetBufferPointer() as *const u8,
                ps_blob.GetBufferSize(),
            );
            device
                .CreatePixelShader(
                    ps_bytes,
                    None,
                    Some(&mut ps),
                )
                .context("CreatePixelShader failed")?;
            let ps = ps.context("missing pixel shader")?;

            let sampler_desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                BorderColor: [0.0, 0.0, 0.0, 0.0],
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
            };
            let mut sampler = None;
            device
                .CreateSamplerState(&sampler_desc, Some(&mut sampler))
                .context("CreateSamplerState failed")?;
            let sampler = sampler.context("missing sampler state")?;

            Ok(Self {
                device,
                context,
                swap_chain,
                rtv,
                vs,
                ps,
                sampler,
                y_tex: None,
                uv_tex: None,
                y_srv: None,
                uv_srv: None,
                frame_width: 0,
                frame_height: 0,
                vsync,
            })
        }
    }

    fn ensure_textures(&mut self, width: u32, height: u32) -> Result<()> {
        if self.frame_width == width && self.frame_height == height && self.y_tex.is_some() {
            return Ok(());
        }

        unsafe {
            self.frame_width = width;
            self.frame_height = height;

            let y_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
            };
            let uv_desc = D3D11_TEXTURE2D_DESC {
                Width: width / 2,
                Height: height / 2,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
            };

            let mut y_tex = None;
            self.device
                .CreateTexture2D(&y_desc, None, Some(&mut y_tex))
                .context("CreateTexture2D Y failed")?;
            let y_tex = y_tex.context("missing y texture")?;

            let mut uv_tex = None;
            self.device
                .CreateTexture2D(&uv_desc, None, Some(&mut uv_tex))
                .context("CreateTexture2D UV failed")?;
            let uv_tex = uv_tex.context("missing uv texture")?;

            let mut y_srv = None;
            self.device
                .CreateShaderResourceView(&y_tex, None, Some(&mut y_srv))
                .context("CreateShaderResourceView Y failed")?;
            let y_srv = y_srv.context("missing y srv")?;

            let mut uv_srv = None;
            self.device
                .CreateShaderResourceView(&uv_tex, None, Some(&mut uv_srv))
                .context("CreateShaderResourceView UV failed")?;
            let uv_srv = uv_srv.context("missing uv srv")?;

            self.y_tex = Some(y_tex);
            self.uv_tex = Some(uv_tex);
            self.y_srv = Some(y_srv);
            self.uv_srv = Some(uv_srv);

            info!(width, height, "video texture resized");
        }
        Ok(())
    }

    fn upload_nv12(&mut self, frame: &DecodedFrame) -> Result<()> {
        self.ensure_textures(frame.width, frame.height)?;
        let y = frame.y_plane();
        let uv = frame.uv_plane();
        let width = frame.width as usize;
        let height = frame.height as usize;

        unsafe {
            let y_tex = self.y_tex.as_ref().context("missing y texture")?;
            let uv_tex = self.uv_tex.as_ref().context("missing uv texture")?;

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(y_tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .context("Map Y failed")?;
            for row in 0..height {
                let src_off = row * width;
                let dst = (mapped.pData as *mut u8).add(row * mapped.RowPitch as usize);
                std::ptr::copy_nonoverlapping(y[src_off..src_off + width].as_ptr(), dst, width);
            }
            self.context.Unmap(y_tex, 0);

            self.context
                .Map(uv_tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .context("Map UV failed")?;
            let uv_h = height / 2;
            for row in 0..uv_h {
                let src_off = row * width;
                let dst = (mapped.pData as *mut u8).add(row * mapped.RowPitch as usize);
                std::ptr::copy_nonoverlapping(uv[src_off..src_off + width].as_ptr(), dst, width);
            }
            self.context.Unmap(uv_tex, 0);
        }
        Ok(())
    }

    fn draw_frame(&mut self) -> Result<()> {
        unsafe {
            let clear = [0.05f32, 0.05f32, 0.08f32, 1.0f32];
            self.context.ClearRenderTargetView(&self.rtv, &clear);
            self.context.OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);

            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.frame_width.max(1) as f32,
                Height: self.frame_height.max(1) as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            self.context.RSSetViewports(Some(&[viewport]));
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY(
                D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST.0,
            ));
            self.context.VSSetShader(&self.vs, None);
            self.context.PSSetShader(&self.ps, None);

            let y_srv = self.y_srv.clone().context("missing y srv")?;
            let uv_srv = self.uv_srv.clone().context("missing uv srv")?;
            self.context.PSSetShaderResources(0, Some(&[Some(y_srv), Some(uv_srv)]));
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.Draw(3, 0);

            let hr = self
                .swap_chain
                .Present(if self.vsync { 1 } else { 0 }, DXGI_PRESENT(0));
            hr.ok().context("swapchain present failed")?;
        }
        Ok(())
    }
}

fn compile_hlsl(src: &[u8], entry: &[u8], target: &[u8]) -> Result<ID3DBlob> {
    unsafe {
        let mut blob: Option<ID3DBlob> = None;
        let mut err_blob: Option<ID3DBlob> = None;
        D3DCompile(
            src.as_ptr() as *const c_void,
            src.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            D3DCOMPILE_ENABLE_STRICTNESS,
            0,
            &mut blob,
            Some(&mut err_blob),
        )
        .map_err(|e| {
            if let Some(err) = err_blob {
                let ptr = err.GetBufferPointer() as *const u8;
                let len = err.GetBufferSize();
                let msg = String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len));
                anyhow::anyhow!("D3DCompile failed: {} ({})", e, msg)
            } else {
                anyhow::anyhow!("D3DCompile failed: {}", e)
            }
        })?;
        blob.context("shader blob is empty")
    }
}
