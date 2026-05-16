use mrd_render::{
    BoxedRenderer, RenderError, RenderFrame, RenderFrameData, RenderPixelFormat, RenderTarget,
    RendererDescriptor, RendererFactory, RendererInstance, RendererSnapshot, RuntimeStatus,
};

const SUPPORTED_FORMATS: &[RenderPixelFormat] =
    &[RenderPixelFormat::Rgb24, RenderPixelFormat::Bgra32];

pub fn opengl_descriptor() -> RendererDescriptor {
    RendererDescriptor {
        id: "opengl",
        runtime_status: RuntimeStatus::RuntimeBacked,
        supported_formats: SUPPORTED_FORMATS,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OpenglRendererFactory;

impl RendererFactory for OpenglRendererFactory {
    fn descriptor(&self) -> RendererDescriptor {
        opengl_descriptor()
    }

    fn create(&self) -> Result<BoxedRenderer, RenderError> {
        Ok(Box::new(OpenglRenderer::new()))
    }
}

#[derive(Debug)]
pub struct OpenglRenderer {
    target_hwnd: Option<isize>,
    #[cfg(windows)]
    surface: Option<WindowsGlSurface>,
    snapshot: RendererSnapshot,
}

impl Default for OpenglRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenglRenderer {
    pub fn new() -> Self {
        Self {
            target_hwnd: None,
            #[cfg(windows)]
            surface: None,
            snapshot: RendererSnapshot {
                attached_to_target: false,
                uploaded_frame_count: 0,
                last_width: 0,
                last_height: 0,
                last_pixel_format: None,
            },
        }
    }

    pub fn target_hwnd(&self) -> Option<isize> {
        self.target_hwnd
    }
}

impl RendererInstance for OpenglRenderer {
    fn attach_target(&mut self, target: RenderTarget) -> Result<(), RenderError> {
        let RenderTarget::WindowHandle(hwnd) = target;
        self.target_hwnd = Some(hwnd);
        #[cfg(windows)]
        {
            self.surface = if hwnd == 0 {
                None
            } else {
                Some(WindowsGlSurface::attach(hwnd)?)
            };
        }
        self.snapshot.attached_to_target = hwnd != 0;
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError> {
        validate_cpu_frame(&frame)?;
        #[cfg(windows)]
        if let Some(surface) = self.surface.as_ref() {
            surface.present_frame(&frame)?;
        }
        self.snapshot.uploaded_frame_count += 1;
        self.snapshot.last_width = frame.width;
        self.snapshot.last_height = frame.height;
        self.snapshot.last_pixel_format = Some(frame.pixel_format);
        Ok(())
    }

    fn snapshot(&self) -> RendererSnapshot {
        self.snapshot.clone()
    }
}

fn validate_cpu_frame(frame: &RenderFrame) -> Result<(), RenderError> {
    if frame.is_shared_texture() {
        return Err(RenderError::Message(
            "OpenGL renderer accepts CPU-backed frames only; D3D11 shared texture input is unsupported"
                .to_string(),
        ));
    }

    match (&frame.pixel_format, &frame.data) {
        (RenderPixelFormat::Rgb24, RenderFrameData::Rgb24(data)) => {
            validate_len(frame.width, frame.height, 3, data.len(), "Rgb24")
        }
        (RenderPixelFormat::Bgra32, RenderFrameData::Bgra32(data)) => {
            validate_len(frame.width, frame.height, 4, data.len(), "Bgra32")
        }
        _ => Err(RenderError::Message(
            "OpenGL renderer only accepts CPU Rgb24 or Bgra32 frame data".to_string(),
        )),
    }
}

fn validate_len(
    width: usize,
    height: usize,
    bytes_per_pixel: usize,
    actual_len: usize,
    label: &str,
) -> Result<(), RenderError> {
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| RenderError::Message(format!("{label} frame dimensions overflow")))?;
    if actual_len != expected_len {
        return Err(RenderError::Message(format!(
            "{label} frame byte length mismatch: expected {expected_len}, got {actual_len}"
        )));
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Debug)]
struct WindowsGlSurface {
    hwnd: windows::Win32::Foundation::HWND,
    hdc: windows::Win32::Graphics::Gdi::HDC,
    context: windows::Win32::Graphics::OpenGL::HGLRC,
}

#[cfg(windows)]
unsafe impl Send for WindowsGlSurface {}

#[cfg(windows)]
impl WindowsGlSurface {
    fn attach(hwnd: isize) -> Result<Self, RenderError> {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::GetDC;
        use windows::Win32::Graphics::OpenGL::{
            wglCreateContext, wglMakeCurrent, ChoosePixelFormat, SetPixelFormat, PFD_DOUBLEBUFFER,
            PFD_DRAW_TO_WINDOW, PFD_FLAGS, PFD_MAIN_PLANE, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA,
            PIXELFORMATDESCRIPTOR,
        };

        unsafe {
            let hwnd = HWND(hwnd);
            let hdc = GetDC(hwnd);
            if hdc.0 == 0 {
                return Err(RenderError::Message(
                    "get OpenGL render target device context failed".to_string(),
                ));
            }

            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_FLAGS(
                    PFD_DRAW_TO_WINDOW.0 | PFD_SUPPORT_OPENGL.0 | PFD_DOUBLEBUFFER.0,
                ),
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                cDepthBits: 0,
                cStencilBits: 0,
                iLayerType: PFD_MAIN_PLANE.0 as u8,
                ..Default::default()
            };
            let pixel_format = ChoosePixelFormat(hdc, &pfd);
            if pixel_format == 0 {
                let _ = windows::Win32::Graphics::Gdi::ReleaseDC(hwnd, hdc);
                return Err(RenderError::Message(
                    "choose OpenGL pixel format failed".to_string(),
                ));
            }
            SetPixelFormat(hdc, pixel_format, &pfd).map_err(|error| {
                let _ = windows::Win32::Graphics::Gdi::ReleaseDC(hwnd, hdc);
                RenderError::Message(format!("set OpenGL pixel format failed: {error}"))
            })?;

            let context = wglCreateContext(hdc).map_err(|error| {
                let _ = windows::Win32::Graphics::Gdi::ReleaseDC(hwnd, hdc);
                RenderError::Message(format!("create OpenGL context failed: {error}"))
            })?;
            wglMakeCurrent(hdc, context).map_err(|error| {
                let _ = windows::Win32::Graphics::OpenGL::wglDeleteContext(context);
                let _ = windows::Win32::Graphics::Gdi::ReleaseDC(hwnd, hdc);
                RenderError::Message(format!("make OpenGL context current failed: {error}"))
            })?;

            Ok(Self { hwnd, hdc, context })
        }
    }

    fn present_frame(&self, frame: &RenderFrame) -> Result<(), RenderError> {
        use std::ffi::c_void;
        use windows::Win32::Graphics::OpenGL::{
            glDrawPixels, glPixelZoom, glRasterPos2f, glViewport, wglMakeCurrent, SwapBuffers,
            GL_BGRA_EXT, GL_RGB, GL_UNSIGNED_BYTE,
        };

        let (format, pixels) = match &frame.data {
            RenderFrameData::Rgb24(data) => (GL_RGB, data.as_ptr()),
            RenderFrameData::Bgra32(data) => (GL_BGRA_EXT, data.as_ptr()),
            _ => {
                return Err(RenderError::Message(
                    "OpenGL present only accepts CPU Rgb24 or Bgra32 frame data".to_string(),
                ))
            }
        };

        let width = i32::try_from(frame.width)
            .map_err(|_| RenderError::Message("OpenGL frame width exceeds i32".to_string()))?;
        let height = i32::try_from(frame.height)
            .map_err(|_| RenderError::Message("OpenGL frame height exceeds i32".to_string()))?;

        unsafe {
            wglMakeCurrent(self.hdc, self.context).map_err(|error| {
                RenderError::Message(format!("make OpenGL context current failed: {error}"))
            })?;
            glViewport(0, 0, width, height);
            glRasterPos2f(-1.0, 1.0);
            glPixelZoom(1.0, -1.0);
            glDrawPixels(
                width,
                height,
                format,
                GL_UNSIGNED_BYTE,
                pixels.cast::<c_void>(),
            );
            SwapBuffers(self.hdc).map_err(|error| {
                RenderError::Message(format!("OpenGL SwapBuffers failed: {error}"))
            })?;
        }

        Ok(())
    }
}

#[cfg(windows)]
impl Drop for WindowsGlSurface {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Graphics::OpenGL::wglMakeCurrent(
                windows::Win32::Graphics::Gdi::HDC(0),
                windows::Win32::Graphics::OpenGL::HGLRC(0),
            );
            let _ = windows::Win32::Graphics::OpenGL::wglDeleteContext(self.context);
            let _ = windows::Win32::Graphics::Gdi::ReleaseDC(self.hwnd, self.hdc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_reports_cpu_backed_opengl_formats() {
        let descriptor = opengl_descriptor();

        assert_eq!(descriptor.id, "opengl");
        assert_eq!(descriptor.runtime_status, RuntimeStatus::RuntimeBacked);
        assert_eq!(
            descriptor.supported_formats,
            &[RenderPixelFormat::Rgb24, RenderPixelFormat::Bgra32]
        );
    }

    #[test]
    fn upload_rgb24_frame_updates_snapshot() {
        let mut renderer = OpenglRenderer::new();
        renderer
            .attach_target(RenderTarget::WindowHandle(0))
            .expect("attach headless OpenGL target");
        renderer
            .upload_frame(RenderFrame::from_rgb24(2, 2, vec![0; 2 * 2 * 3]))
            .expect("upload RGB frame");

        let snapshot = renderer.snapshot();
        assert!(!snapshot.attached_to_target);
        assert_eq!(renderer.target_hwnd(), Some(0));
        assert_eq!(snapshot.uploaded_frame_count, 1);
        assert_eq!(snapshot.last_width, 2);
        assert_eq!(snapshot.last_height, 2);
        assert_eq!(snapshot.last_pixel_format, Some(RenderPixelFormat::Rgb24));
    }

    #[test]
    fn upload_rejects_truncated_cpu_frame() {
        let mut renderer = OpenglRenderer::new();
        let error = renderer
            .upload_frame(RenderFrame::from_bgra32(2, 2, vec![0; 7]))
            .expect_err("truncated BGRA frame should fail");

        assert!(error.to_string().contains("byte length mismatch"));
        assert_eq!(renderer.snapshot().uploaded_frame_count, 0);
    }

    #[cfg(windows)]
    #[test]
    fn upload_rejects_d3d11_shared_texture_input() {
        let mut renderer = OpenglRenderer::new();
        let error = renderer
            .upload_frame(RenderFrame::from_d3d11_shared_nv12(1920, 1080, 1, 2))
            .expect_err("D3D11 shared textures are not OpenGL CPU frames");

        assert!(error.to_string().contains("CPU-backed frames only"));
    }
}
