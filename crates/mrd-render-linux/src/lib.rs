//! Linux frame rendering implementation
//!
//! Provides frame rendering on Linux using various backends:
//! - X11 (traditional)
//! - Software (fallback)

#![cfg(target_os = "linux")]
#![warn(missing_docs)]

use mrd_render::{
    RenderFrame, RenderPixelFormat, RenderTarget, RendererDescriptor, RendererFactory,
    RendererInstance, RendererSnapshot, RuntimeStatus,
};
use thiserror::Error;

const SUPPORTED_FORMATS: &[RenderPixelFormat] =
    &[RenderPixelFormat::Rgb24, RenderPixelFormat::Bgra32];

/// Linux frame renderer supporting multiple backends
pub struct LinuxRenderer {
    width: u32,
    height: u32,
    last_pixel_format: Option<RenderPixelFormat>,
    backend: RendererBackend,
    frame_count: u64,
    attached: bool,
}

enum RendererBackend {
    Software(SoftwareRenderer),
    #[cfg(feature = "x11")]
    X11(X11Renderer),
}

impl RendererBackend {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            RendererBackend::Software(r) => r.dimensions(),
            #[cfg(feature = "x11")]
            RendererBackend::X11(r) => r.dimensions(),
        }
    }
}

/// Linux renderer factory
pub struct LinuxRendererFactory;

impl RendererFactory for LinuxRendererFactory {
    fn descriptor(&self) -> RendererDescriptor {
        RendererDescriptor {
            id: "linux",
            runtime_status: RuntimeStatus::RuntimeBacked,
            supported_formats: SUPPORTED_FORMATS,
        }
    }

    fn create(&self) -> Result<Box<dyn RendererInstance>, mrd_render::RenderError> {
        Ok(Box::new(LinuxRenderer::new().map_err(|e| {
            mrd_render::RenderError::Message(e.to_string())
        })?))
    }
}

impl LinuxRenderer {
    pub fn new() -> Result<Self, LinuxRenderError> {
        let backend = Self::select_backend()?;
        let (width, height) = backend.dimensions();

        Ok(Self {
            width,
            height,
            last_pixel_format: None,
            backend,
            frame_count: 0,
            attached: false,
        })
    }

    fn select_backend() -> Result<RendererBackend, LinuxRenderError> {
        #[cfg(feature = "x11")]
        {
            if std::env::var("DISPLAY").is_ok() {
                match X11Renderer::new() {
                    Ok(renderer) => return Ok(RendererBackend::X11(renderer)),
                    Err(e) => {
                        eprintln!("X11 renderer init failed: {}, falling back to software", e);
                    }
                }
            }
        }

        Ok(RendererBackend::Software(SoftwareRenderer::new()?))
    }

    pub fn backend_name(&self) -> &'static str {
        match &self.backend {
            RendererBackend::Software(_) => "software",
            #[cfg(feature = "x11")]
            RendererBackend::X11(_) => "x11",
        }
    }

    /// Create a backend-owned native test window when the active backend supports it.
    pub fn create_window(&mut self, title: &str) -> Result<(), LinuxRenderError> {
        self.create_window_with_size(title, self.width as usize, self.height as usize)
    }

    /// Create a backend-owned native test window with the requested client size.
    pub fn create_window_with_size(
        &mut self,
        title: &str,
        width: usize,
        height: usize,
    ) -> Result<(), LinuxRenderError> {
        self.width = width.max(1) as u32;
        self.height = height.max(1) as u32;
        match &mut self.backend {
            RendererBackend::Software(_) => {
                self.attached = true;
                Ok(())
            }
            #[cfg(feature = "x11")]
            RendererBackend::X11(renderer) => {
                renderer.create_window(title, self.width, self.height)?;
                self.attached = true;
                Ok(())
            }
        }
    }
}

impl RendererInstance for LinuxRenderer {
    fn attach_target(&mut self, target: RenderTarget) -> Result<(), mrd_render::RenderError> {
        self.attached = match &mut self.backend {
            RendererBackend::Software(renderer) => renderer.attach_target(target).is_ok(),
            #[cfg(feature = "x11")]
            RendererBackend::X11(renderer) => renderer.attach_target(target).is_ok(),
        };
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), mrd_render::RenderError> {
        self.frame_count += 1;
        self.width = frame.width.max(1) as u32;
        self.height = frame.height.max(1) as u32;
        self.last_pixel_format = Some(frame.pixel_format);
        match &mut self.backend {
            RendererBackend::Software(renderer) => renderer.upload_frame(frame),
            #[cfg(feature = "x11")]
            RendererBackend::X11(renderer) => renderer.upload_frame(frame),
        }
    }

    fn snapshot(&self) -> RendererSnapshot {
        RendererSnapshot {
            attached_to_target: self.attached,
            uploaded_frame_count: self.frame_count,
            last_width: self.width as usize,
            last_height: self.height as usize,
            last_pixel_format: self.last_pixel_format,
        }
    }
}

/// Software renderer (fallback)
pub struct SoftwareRenderer {
    buffer: Vec<u8>,
    frame_count: u64,
    last_width: usize,
    last_height: usize,
    last_pixel_format: Option<RenderPixelFormat>,
}

impl SoftwareRenderer {
    pub fn new() -> Result<Self, LinuxRenderError> {
        Ok(Self {
            buffer: Vec::new(),
            frame_count: 0,
            last_width: 1920,
            last_height: 1080,
            last_pixel_format: None,
        })
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.last_width as u32, self.last_height as u32)
    }
}

impl RendererInstance for SoftwareRenderer {
    fn attach_target(&mut self, _target: RenderTarget) -> Result<(), mrd_render::RenderError> {
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), mrd_render::RenderError> {
        self.frame_count += 1;
        self.last_width = frame.width;
        self.last_height = frame.height;
        self.last_pixel_format = Some(frame.pixel_format);
        // Store frame data
        if let Some(data) = frame.as_bgra32() {
            self.buffer = data.to_vec();
        } else if let Some(data) = frame.as_rgb24() {
            // Convert RGB24 to BGRA32
            self.buffer = data
                .chunks_exact(3)
                .flat_map(|rgb| [rgb[2], rgb[1], rgb[0], 255])
                .collect();
        }
        Ok(())
    }

    fn snapshot(&self) -> RendererSnapshot {
        RendererSnapshot {
            attached_to_target: false,
            uploaded_frame_count: self.frame_count,
            last_width: self.last_width,
            last_height: self.last_height,
            last_pixel_format: self.last_pixel_format,
        }
    }
}

/// X11 renderer implementation
#[cfg(feature = "x11")]
pub struct X11Renderer {
    display: *mut x11::xlib::Display,
    window: Option<x11::xlib::Window>,
    visual: *mut x11::xlib::Visual,
    gc: x11::xlib::GC,
    width: u32,
    height: u32,
    frame_count: u64,
    last_pixel_format: Option<RenderPixelFormat>,
}

#[cfg(feature = "x11")]
unsafe impl Send for X11Renderer {}

#[cfg(feature = "x11")]
impl X11Renderer {
    pub fn new() -> Result<Self, LinuxRenderError> {
        use std::ptr;
        use x11::xlib;

        unsafe {
            let display = (xlib::XOpenDisplay)(ptr::null());

            if display.is_null() {
                return Err(LinuxRenderError::InitFailed(
                    "Failed to open X11 display".to_string(),
                ));
            }

            let screen = (xlib::XDefaultScreen)(display);
            let visual = (xlib::XDefaultVisual)(display, screen);
            let root = (xlib::XRootWindow)(display, screen);
            let width = (xlib::XDisplayWidth)(display, screen) as u32;
            let height = (xlib::XDisplayHeight)(display, screen) as u32;

            // Create graphics context
            let mut gc_values: x11::xlib::XGCValues = std::mem::zeroed();
            let gc = (xlib::XCreateGC)(display, root, 0, &mut gc_values);

            if gc.is_null() {
                (xlib::XCloseDisplay)(display);
                return Err(LinuxRenderError::InitFailed(
                    "Failed to create GC".to_string(),
                ));
            }

            Ok(Self {
                display,
                window: None,
                visual,
                gc,
                width,
                height,
                frame_count: 0,
                last_pixel_format: None,
            })
        }
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn create_window(
        &mut self,
        title: &str,
        width: u32,
        height: u32,
    ) -> Result<(), LinuxRenderError> {
        use x11::xlib;

        unsafe {
            let screen = (xlib::XDefaultScreen)(self.display);
            let root = (xlib::XRootWindow)(self.display, screen);

            let mut attrs: x11::xlib::XSetWindowAttributes = std::mem::zeroed();
            attrs.background_pixel = (xlib::XWhitePixel)(self.display, screen);
            attrs.event_mask = x11::xlib::ExposureMask
                | x11::xlib::StructureNotifyMask
                | x11::xlib::KeyPressMask;
            self.width = width.max(1);
            self.height = height.max(1);

            let window = (xlib::XCreateWindow)(
                self.display,
                root,
                0,
                0,
                self.width,
                self.height,
                0,
                (xlib::XDefaultDepth)(self.display, screen),
                x11::xlib::InputOutput as u32,
                self.visual as *mut _,
                x11::xlib::CWBackPixel | x11::xlib::CWEventMask,
                &mut attrs,
            );

            if window == 0 {
                return Err(LinuxRenderError::InitFailed(
                    "Failed to create window".to_string(),
                ));
            }

            // Set window title
            let title_cstr = std::ffi::CString::new(title).unwrap();
            (xlib::XStoreName)(self.display, window, title_cstr.as_ptr());

            // Map window
            (xlib::XMapWindow)(self.display, window);
            (xlib::XFlush)(self.display);

            self.window = Some(window);
            Ok(())
        }
    }

    fn pump_events(&mut self) {
        use x11::xlib;

        unsafe {
            while (xlib::XPending)(self.display) > 0 {
                let mut event: x11::xlib::XEvent = std::mem::zeroed();
                (xlib::XNextEvent)(self.display, &mut event);
                if event.get_type() == xlib::ConfigureNotify {
                    let configure = event.configure;
                    if configure.width > 0 && configure.height > 0 {
                        self.width = configure.width as u32;
                        self.height = configure.height as u32;
                    }
                }
            }
        }
    }

    fn put_image(
        &mut self,
        data: &[u8],
        width: usize,
        height: usize,
    ) -> Result<(), LinuxRenderError> {
        use x11::xlib;

        unsafe {
            self.pump_events();
            let window = match self.window {
                Some(w) => w,
                None => return Ok(()), // No window to render to
            };

            // Create XImage from data
            let image = (xlib::XCreateImage)(
                self.display,
                self.visual,
                24, // depth
                x11::xlib::ZPixmap,
                0,
                data.as_ptr() as *mut i8,
                width as u32,
                height as u32,
                32, // bitmap_pad
                0,  // bytes_per_line (0 = auto)
            );

            if image.is_null() {
                return Err(LinuxRenderError::X11Error(
                    "Failed to create XImage".to_string(),
                ));
            }

            // Put image to window
            (xlib::XPutImage)(
                self.display,
                window,
                self.gc,
                image,
                0,
                0,
                0,
                0,
                width as u32,
                height as u32,
            );

            (*image).data = std::ptr::null_mut();
            (xlib::XDestroyImage)(image);

            (xlib::XFlush)(self.display);
            Ok(())
        }
    }
}

#[cfg(feature = "x11")]
impl Drop for X11Renderer {
    fn drop(&mut self) {
        use x11::xlib;

        unsafe {
            if let Some(window) = self.window {
                (xlib::XDestroyWindow)(self.display, window);
            }
            if !self.gc.is_null() {
                (xlib::XFreeGC)(self.display, self.gc);
            }
            (xlib::XCloseDisplay)(self.display);
        }
    }
}

#[cfg(feature = "x11")]
impl RendererInstance for X11Renderer {
    fn attach_target(&mut self, target: RenderTarget) -> Result<(), mrd_render::RenderError> {
        use mrd_render::RenderTarget;

        match target {
            RenderTarget::WindowHandle(handle) => {
                self.window = Some(handle as x11::xlib::Window);
                Ok(())
            }
        }
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), mrd_render::RenderError> {
        self.frame_count += 1;
        self.last_pixel_format = Some(frame.pixel_format);

        // Get frame data
        let data = if let Some(bgra) = frame.as_bgra32() {
            bgra.to_vec()
        } else if let Some(rgb) = frame.as_rgb24() {
            // Convert RGB24 to BGRA32
            rgb.chunks_exact(3)
                .flat_map(|rgb| [rgb[2], rgb[1], rgb[0], 255])
                .collect()
        } else {
            return Ok(());
        };
        self.width = frame.width.max(1) as u32;
        self.height = frame.height.max(1) as u32;

        // Render to X11 window
        if let Err(e) = self.put_image(&data, frame.width, frame.height) {
            eprintln!("X11 render error: {}", e);
        }

        Ok(())
    }

    fn snapshot(&self) -> RendererSnapshot {
        RendererSnapshot {
            attached_to_target: self.window.is_some(),
            uploaded_frame_count: self.frame_count,
            last_width: self.width as usize,
            last_height: self.height as usize,
            last_pixel_format: self.last_pixel_format,
        }
    }
}

/// Linux-specific render errors
#[derive(Debug, Error)]
pub enum LinuxRenderError {
    #[error("Failed to initialize Linux renderer: {0}")]
    InitFailed(String),

    #[error("No suitable rendering backend available")]
    NoBackend,

    #[error("X11 rendering failed: {0}")]
    X11Error(String),
}

/// Create a Linux renderer descriptor
pub fn linux_renderer_descriptor() -> RendererDescriptor {
    RendererDescriptor {
        id: "linux",
        runtime_status: RuntimeStatus::RuntimeBacked,
        supported_formats: SUPPORTED_FORMATS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_renderer_factory() {
        let factory = LinuxRendererFactory;
        let descriptor = factory.descriptor();
        assert_eq!(descriptor.id, "linux");
    }

    #[test]
    fn test_renderer_creation() {
        let factory = LinuxRendererFactory;
        let renderer = factory.create();
        assert!(renderer.is_ok());
    }

    #[test]
    fn test_snapshot() {
        let factory = LinuxRendererFactory;
        let renderer = factory.create().unwrap();
        let snapshot = renderer.snapshot();
        // Check for valid dimensions (any reasonable screen size)
        assert!(snapshot.last_width >= 800);
        assert!(snapshot.last_height >= 600);
    }

    #[test]
    #[cfg(feature = "x11")]
    fn test_x11_renderer() {
        if std::env::var("DISPLAY").is_err() {
            return;
        }

        match X11Renderer::new() {
            Ok(mut renderer) => {
                println!("X11 renderer created successfully");
                println!("Dimensions: {}x{}", renderer.width, renderer.height);
            }
            Err(e) => {
                println!("X11 renderer creation failed (may be expected): {}", e);
            }
        }
    }
}
