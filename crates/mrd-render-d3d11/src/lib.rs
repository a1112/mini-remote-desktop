use mrd_render::{
    d3d11_descriptor, BoxedRenderer, RenderError, RenderFrame, RenderPixelFormat, RenderTarget,
    RendererDescriptor, RendererFactory, RendererInstance, RendererSnapshot,
};
#[cfg(windows)]
use windows::core::ComInterface;

pub struct D3d11RendererFactory;

impl RendererFactory for D3d11RendererFactory {
    fn descriptor(&self) -> RendererDescriptor {
        d3d11_descriptor()
    }

    fn create(&self) -> Result<BoxedRenderer, RenderError> {
        Ok(Box::new(D3d11Renderer::new()?))
    }
}

#[cfg(windows)]
struct RenderSurface {
    swap_chain: windows::Win32::Graphics::Dxgi::IDXGISwapChain1,
    back_buffer: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    render_target_view: windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView,
}

pub struct D3d11Renderer {
    #[cfg(windows)]
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    #[cfg(windows)]
    context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    #[cfg(windows)]
    surface: Option<RenderSurface>,
    attached_to_target: bool,
    uploaded_frame_count: u64,
    last_width: usize,
    last_height: usize,
    last_pixel_format: Option<RenderPixelFormat>,
}

impl D3d11Renderer {
    pub fn new() -> Result<Self, RenderError> {
        #[cfg(windows)]
        {
            use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
            use windows::Win32::Graphics::Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
                ID3D11Device, ID3D11DeviceContext,
            };

            let mut device = None::<ID3D11Device>;
            let mut context = None::<ID3D11DeviceContext>;

            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    None,
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
            }
            .map_err(|error| RenderError::Message(format!("创建 D3D11 设备失败: {error}")))?;

            let device = device.ok_or_else(|| RenderError::Message("缺少 D3D11 device".into()))?;
            let context =
                context.ok_or_else(|| RenderError::Message("缺少 D3D11 device context".into()))?;

            Ok(Self {
                device,
                context,
                surface: None,
                attached_to_target: false,
                uploaded_frame_count: 0,
                last_width: 0,
                last_height: 0,
                last_pixel_format: None,
            })
        }

        #[cfg(not(windows))]
        {
            Err(RenderError::Message(
                "d3d11 renderer 仅支持 Windows".to_string(),
            ))
        }
    }

    #[cfg(windows)]
    fn attach_window_surface(
        &mut self,
        window_handle: isize,
    ) -> Result<Option<RenderSurface>, RenderError> {
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::Graphics::Direct3D11::{ID3D11RenderTargetView, ID3D11Texture2D};
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
        };
        use windows::Win32::Graphics::Dxgi::{
            DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD,
            DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
        };
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

        if window_handle == 0 {
            return Ok(None);
        }

        let hwnd = HWND(window_handle);
        let mut rect = RECT::default();
        unsafe { GetClientRect(hwnd, &mut rect) }
            .map_err(|error| RenderError::Message(format!("读取窗口大小失败: {error}")))?;
        let width = (rect.right - rect.left).max(1) as u32;
        let height = (rect.bottom - rect.top).max(1) as u32;

        let dxgi_device: IDXGIDevice = self
            .device
            .cast()
            .map_err(|error| RenderError::Message(format!("转换 IDXGIDevice 失败: {error}")))?;
        let adapter = unsafe { dxgi_device.GetAdapter() }
            .map_err(|error| RenderError::Message(format!("获取 DXGI adapter 失败: {error}")))?;
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent() }
            .map_err(|error| RenderError::Message(format!("获取 DXGI factory 失败: {error}")))?;
        let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };

        let swap_chain: IDXGISwapChain1 = unsafe {
            factory.CreateSwapChainForHwnd(
                &self.device,
                hwnd,
                &swap_chain_desc,
                None,
                None::<&windows::Win32::Graphics::Dxgi::IDXGIOutput>,
            )
        }
        .map_err(|error| RenderError::Message(format!("创建 SwapChain 失败: {error}")))?;

        let back_buffer: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0) }
            .map_err(|error| RenderError::Message(format!("获取 back buffer 失败: {error}")))?;
        let mut render_target_view = None::<ID3D11RenderTargetView>;
        unsafe { self.device.CreateRenderTargetView(&back_buffer, None, Some(&mut render_target_view)) }
            .map_err(|error| RenderError::Message(format!("创建 RTV 失败: {error}")))?;
        let render_target_view =
            render_target_view.ok_or_else(|| RenderError::Message("缺少 render target view".into()))?;

        Ok(Some(RenderSurface {
            swap_chain,
            back_buffer,
            render_target_view,
        }))
    }

    #[cfg(windows)]
    fn average_clear_color(frame: &RenderFrame) -> [f32; 4] {
        if frame.data.is_empty() {
            return [0.05, 0.05, 0.05, 1.0];
        }

        let mut r: u64 = 0;
        let mut g: u64 = 0;
        let mut b: u64 = 0;
        let mut pixels: u64 = 0;

        for chunk in frame.data.chunks_exact(3) {
            r += chunk[0] as u64;
            g += chunk[1] as u64;
            b += chunk[2] as u64;
            pixels += 1;
        }

        if pixels == 0 {
            return [0.05, 0.05, 0.05, 1.0];
        }

        [
            (r as f32 / pixels as f32) / 255.0,
            (g as f32 / pixels as f32) / 255.0,
            (b as f32 / pixels as f32) / 255.0,
            1.0,
        ]
    }

    #[cfg(windows)]
    fn present_clear_frame(&self, frame: &RenderFrame) -> Result<(), RenderError> {
        let Some(surface) = self.surface.as_ref() else {
            return Ok(());
        };

        let clear = Self::average_clear_color(frame);
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(surface.render_target_view.clone())]), None);
            self.context
                .ClearRenderTargetView(&surface.render_target_view, &clear);
            surface
                .swap_chain
                .Present(1, 0)
                .ok()
                .map_err(|error| RenderError::Message(format!("present 失败: {error}")))?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn rgb24_to_bgra(frame: &RenderFrame) -> Result<Vec<u8>, RenderError> {
        let expected = frame
            .width
            .checked_mul(frame.height)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or_else(|| RenderError::Message("frame size overflow".into()))?;
        if frame.data.len() != expected {
            return Err(RenderError::Message(format!(
                "Rgb24 frame bytes mismatch: expected {expected}, got {}",
                frame.data.len()
            )));
        }

        let mut bgra = Vec::with_capacity(frame.width * frame.height * 4);
        for chunk in frame.data.chunks_exact(3) {
            bgra.push(chunk[2]);
            bgra.push(chunk[1]);
            bgra.push(chunk[0]);
            bgra.push(255);
        }
        Ok(bgra)
    }

    #[cfg(windows)]
    fn present_uploaded_frame(&self, frame: &RenderFrame) -> Result<(), RenderError> {
        let Some(surface) = self.surface.as_ref() else {
            return Ok(());
        };

        let bgra = Self::rgb24_to_bgra(frame)?;
        let row_pitch = frame
            .width
            .checked_mul(4)
            .ok_or_else(|| RenderError::Message("row pitch overflow".into()))?
            as u32;

        unsafe {
            self.context.UpdateSubresource(
                &surface.back_buffer,
                0,
                None,
                bgra.as_ptr() as *const core::ffi::c_void,
                row_pitch,
                0,
            );
            self.context
                .OMSetRenderTargets(Some(&[Some(surface.render_target_view.clone())]), None);
            surface
                .swap_chain
                .Present(1, 0)
                .ok()
                .map_err(|error| RenderError::Message(format!("present 失败: {error}")))?;
        }
        Ok(())
    }
}

impl RendererInstance for D3d11Renderer {
    fn attach_target(&mut self, target: RenderTarget) -> Result<(), RenderError> {
        #[cfg(windows)]
        {
            self.surface = match target {
                RenderTarget::WindowHandle(window_handle) => self.attach_window_surface(window_handle)?,
            };
        }

        self.attached_to_target = true;
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError> {
        if frame.pixel_format != RenderPixelFormat::Rgb24 {
            return Err(RenderError::Message("d3d11 backend 当前只支持 Rgb24".into()));
        }

        #[cfg(windows)]
        if self.surface.is_some() {
            self.present_uploaded_frame(&frame)?;
        } else {
            self.present_clear_frame(&frame)?;
        }

        self.uploaded_frame_count += 1;
        self.last_width = frame.width;
        self.last_height = frame.height;
        self.last_pixel_format = Some(frame.pixel_format);
        Ok(())
    }

    fn snapshot(&self) -> RendererSnapshot {
        RendererSnapshot {
            attached_to_target: self.attached_to_target,
            uploaded_frame_count: self.uploaded_frame_count,
            last_width: self.last_width,
            last_height: self.last_height,
            last_pixel_format: self.last_pixel_format,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::D3d11RendererFactory;
    use mrd_render::{RenderFrame, RenderPixelFormat, RenderTarget, RendererFactory};

    #[cfg(windows)]
    #[test]
    fn d3d11_factory_creates_backend_and_tracks_uploads() {
        let factory = D3d11RendererFactory;
        let mut renderer = factory.create().expect("d3d11 renderer");

        renderer
            .attach_target(RenderTarget::WindowHandle(0))
            .expect("attach target");
        renderer
            .upload_frame(RenderFrame {
                width: 16,
                height: 16,
                pixel_format: RenderPixelFormat::Rgb24,
                data: vec![128; 16 * 16 * 3],
            })
            .expect("upload frame");

        let snapshot = renderer.snapshot();
        assert!(snapshot.attached_to_target);
        assert_eq!(snapshot.uploaded_frame_count, 1);
        assert_eq!(snapshot.last_width, 16);
        assert_eq!(snapshot.last_height, 16);
        assert_eq!(snapshot.last_pixel_format, Some(RenderPixelFormat::Rgb24));
    }

    #[cfg(not(windows))]
    #[test]
    fn d3d11_factory_reports_platform_error_off_windows() {
        let factory = D3d11RendererFactory;
        let error = factory.create().expect_err("platform error");

        assert!(error.to_string().contains("Windows"));
    }
}
