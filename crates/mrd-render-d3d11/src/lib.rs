use mrd_render::{
    d3d11_descriptor, BoxedRenderer, RenderError, RenderFrame, RenderPixelFormat, RenderTarget,
    RendererDescriptor, RendererFactory, RendererInstance, RendererSnapshot,
};

pub struct D3d11RendererFactory;

impl RendererFactory for D3d11RendererFactory {
    fn descriptor(&self) -> RendererDescriptor {
        d3d11_descriptor()
    }

    fn create(&self) -> Result<BoxedRenderer, RenderError> {
        Ok(Box::new(D3d11Renderer::new()?))
    }
}

pub struct D3d11Renderer {
    #[cfg(windows)]
    _device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    #[cfg(windows)]
    _context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
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
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_SDK_VERSION,
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
                _device: device,
                _context: context,
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
}

impl RendererInstance for D3d11Renderer {
    fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
        self.attached_to_target = true;
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError> {
        if frame.pixel_format != RenderPixelFormat::Rgb24 {
            return Err(RenderError::Message("d3d11 backend 当前只支持 Rgb24".into()));
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
