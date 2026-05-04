#![allow(unexpected_cfgs)]

#[cfg(target_os = "macos")]
use metal::foreign_types::ForeignType;
use mrd_render::{
    BoxedRenderer, RenderError, RenderFrame, RenderFrameData, RenderPixelFormat, RenderTarget,
    RendererDescriptor, RendererFactory, RendererInstance, RendererSnapshot, RuntimeStatus,
};

const MACOS_SUPPORTED_FORMATS: &[RenderPixelFormat] =
    &[RenderPixelFormat::Rgb24, RenderPixelFormat::Bgra32];

pub struct MacosRendererFactory;

impl RendererFactory for MacosRendererFactory {
    fn descriptor(&self) -> RendererDescriptor {
        RendererDescriptor {
            id: "metal",
            runtime_status: RuntimeStatus::RuntimeBacked,
            supported_formats: MACOS_SUPPORTED_FORMATS,
        }
    }

    fn create(&self) -> Result<BoxedRenderer, RenderError> {
        #[cfg(target_os = "macos")]
        {
            Ok(Box::new(MacosMetalRenderer::new()?))
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(RenderError::Message(
                "Metal renderer is only available on macOS".to_string(),
            ))
        }
    }
}

#[cfg(target_os = "macos")]
pub struct MacosMetalRenderer {
    device: metal::Device,
    command_queue: metal::CommandQueue,
    layer: Option<metal::MetalLayer>,
    target_ns_view: Option<isize>,
    texture: Option<metal::Texture>,
    texture_width: usize,
    texture_height: usize,
    scratch_bgra: Vec<u8>,
    attached_to_target: bool,
    drawable_width: usize,
    drawable_height: usize,
    uploaded_frame_count: u64,
    last_width: usize,
    last_height: usize,
    last_pixel_format: Option<RenderPixelFormat>,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacosMetalRenderer {}

#[cfg(target_os = "macos")]
impl MacosMetalRenderer {
    pub fn new() -> Result<Self, RenderError> {
        let device = metal::Device::system_default()
            .ok_or_else(|| RenderError::Message("Metal device is not available".to_string()))?;
        let command_queue = device.new_command_queue();

        Ok(Self {
            device,
            command_queue,
            layer: None,
            target_ns_view: None,
            texture: None,
            texture_width: 0,
            texture_height: 0,
            scratch_bgra: Vec::new(),
            attached_to_target: false,
            drawable_width: 0,
            drawable_height: 0,
            uploaded_frame_count: 0,
            last_width: 0,
            last_height: 0,
            last_pixel_format: None,
        })
    }

    fn ensure_texture(&mut self, width: usize, height: usize) {
        if self.texture.is_some() && self.texture_width == width && self.texture_height == height {
            return;
        }

        let descriptor = metal::TextureDescriptor::new();
        descriptor.set_texture_type(metal::MTLTextureType::D2);
        descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        descriptor.set_width(width as u64);
        descriptor.set_height(height as u64);
        descriptor.set_storage_mode(metal::MTLStorageMode::Shared);
        descriptor.set_usage(metal::MTLTextureUsage::Unknown);

        self.texture = Some(self.device.new_texture(&descriptor));
        self.texture_width = width;
        self.texture_height = height;
    }

    fn upload_bgra(&mut self, width: usize, height: usize, data: &[u8]) -> Result<(), RenderError> {
        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| RenderError::Message("Metal frame size overflow".to_string()))?;
        if data.len() != expected {
            return Err(RenderError::Message(format!(
                "Metal BGRA frame bytes mismatch: expected {expected}, got {}",
                data.len()
            )));
        }

        self.ensure_texture(width, height);
        let texture = self
            .texture
            .as_ref()
            .ok_or_else(|| RenderError::Message("Metal texture was not created".to_string()))?;
        let region = metal::MTLRegion::new_2d(0, 0, width as u64, height as u64);
        texture.replace_region(region, 0, data.as_ptr().cast(), (width * 4) as u64);
        Ok(())
    }

    fn attach_ns_view(&mut self, ns_view: isize) -> Result<(), RenderError> {
        if ns_view == 0 {
            return Err(RenderError::Message(
                "Metal renderer requires a non-null NSView render target".to_string(),
            ));
        }

        let device = self.device.clone();
        let (layer, drawable_width, drawable_height) = run_on_main_thread_sync(move || unsafe {
            use objc::runtime::Object;

            let mut layer = metal::MetalLayer::new();
            layer.set_device(&device);
            layer.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
            layer.set_presents_with_transaction(false);
            layer.set_framebuffer_only(false);
            layer.set_display_sync_enabled(true);
            layer.set_maximum_drawable_count(3);
            layer.set_masks_to_bounds(true);
            layer.set_opaque(true);
            layer.remove_all_animations();

            let layer_object = layer.as_mut() as *mut _ as *mut Object as usize;
            let (drawable_width, drawable_height) =
                sync_layer_geometry_on_main(layer_object, ns_view, true)?;
            layer.set_drawable_size(core_graphics_types::geometry::CGSize::new(
                drawable_width as f64,
                drawable_height as f64,
            ));
            Ok((layer, drawable_width, drawable_height))
        })
        .map_err(RenderError::Message)?;

        self.layer = Some(layer);
        self.target_ns_view = Some(ns_view);
        self.drawable_width = drawable_width;
        self.drawable_height = drawable_height;
        Ok(())
    }

    fn present_if_attached(&mut self, width: usize, height: usize) {
        objc::rc::autoreleasepool(|| self.present_if_attached_inner(width, height));
    }

    fn present_if_attached_inner(&mut self, width: usize, height: usize) {
        let Some(texture) = self.texture.as_ref().cloned() else {
            return;
        };
        let Some(layer) = self.layer.as_ref().cloned() else {
            return;
        };

        let ns_view = self.target_ns_view;
        let layer_object = ns_view.map(|_| layer.as_ptr() as *mut objc::runtime::Object as usize);
        let command_queue = self.command_queue.clone();
        if let Ok(drawable_size) = run_on_main_thread_sync(move || {
            objc::rc::autoreleasepool(|| {
                let drawable_size = match (layer_object, ns_view) {
                    (Some(layer_object), Some(ns_view)) => unsafe {
                        sync_layer_geometry_on_main(layer_object, ns_view, false)?
                    },
                    _ => (width, height),
                };
                layer.set_drawable_size(core_graphics_types::geometry::CGSize::new(
                    drawable_size.0 as f64,
                    drawable_size.1 as f64,
                ));
                let Some(drawable) = layer.next_drawable() else {
                    return Ok(drawable_size);
                };
                let drawable_texture = drawable.texture();
                let dst_width = drawable_texture.width() as usize;
                let dst_height = drawable_texture.height() as usize;
                let Some((copy_width, copy_height)) =
                    copy_region_size(width, height, dst_width, dst_height)
                else {
                    return Ok(drawable_size);
                };

                let command_buffer = command_queue.new_command_buffer();
                let blit = command_buffer.new_blit_command_encoder();
                blit.copy_from_texture(
                    &texture,
                    0,
                    0,
                    metal::MTLOrigin { x: 0, y: 0, z: 0 },
                    metal::MTLSize::new(copy_width as u64, copy_height as u64, 1),
                    drawable_texture,
                    0,
                    0,
                    metal::MTLOrigin { x: 0, y: 0, z: 0 },
                );
                blit.end_encoding();
                command_buffer.present_drawable(drawable);
                command_buffer.commit();
                command_buffer.wait_until_completed();
                Ok(drawable_size)
            })
        }) {
            self.drawable_width = drawable_size.0;
            self.drawable_height = drawable_size.1;
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn sync_layer_geometry_on_main(
    layer_object: usize,
    ns_view: isize,
    attach_to_view: bool,
) -> Result<(usize, usize), String> {
    use objc::{
        msg_send,
        runtime::{Object, YES},
        sel, sel_impl,
    };

    let view = ns_view as *mut Object;
    let layer_object = layer_object as *mut Object;
    if view.is_null() || layer_object.is_null() {
        return Err("macOS render layer target became null".to_string());
    }

    let _: () = msg_send![view, setWantsLayer: YES];
    let bounds: core_graphics_types::geometry::CGRect = msg_send![view, bounds];
    let window: *mut Object = msg_send![view, window];
    let contents_scale = if window.is_null() {
        1.0
    } else {
        msg_send![window, backingScaleFactor]
    };
    let _: () = msg_send![layer_object, setFrame: bounds];
    let _: () = msg_send![layer_object, setContentsScale: contents_scale];
    if attach_to_view {
        let _: () = msg_send![layer_object, setNeedsDisplayOnBoundsChange: YES];
        let _: () = msg_send![layer_object, setZPosition: 1000.0_f64];
        let _: () = msg_send![view, setLayer: layer_object];
    }
    let _: () = msg_send![view, setNeedsDisplay: YES];

    Ok((
        scaled_drawable_dimension(bounds.size.width, contents_scale),
        scaled_drawable_dimension(bounds.size.height, contents_scale),
    ))
}

#[cfg(target_os = "macos")]
fn scaled_drawable_dimension(points: f64, scale: f64) -> usize {
    let pixels = (points * scale).round();
    if pixels.is_finite() && pixels > 0.0 {
        pixels as usize
    } else {
        1
    }
}

#[cfg(target_os = "macos")]
fn copy_region_size(
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Option<(usize, usize)> {
    let width = src_width.min(dst_width);
    let height = src_height.min(dst_height);
    if width == 0 || height == 0 {
        None
    } else {
        Some((width, height))
    }
}

#[cfg(target_os = "macos")]
fn run_on_main_thread_sync<T, F>(f: F) -> Result<T, String>
where
    T: Send,
    F: FnOnce() -> Result<T, String> + Send,
{
    if unsafe { pthread_main_np() } != 0 {
        return f();
    }

    let mut result: Option<Result<T, String>> = None;
    dispatch2::DispatchQueue::main().exec_sync(|| {
        result = Some(f());
    });
    result.unwrap_or_else(|| Err("macOS main-thread task did not return".to_string()))
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_main_np() -> std::ffi::c_int;
}

#[cfg(target_os = "macos")]
impl RendererInstance for MacosMetalRenderer {
    fn attach_target(&mut self, target: RenderTarget) -> Result<(), RenderError> {
        let RenderTarget::WindowHandle(window_handle) = target;
        self.attach_ns_view(window_handle)?;
        self.attached_to_target = self.layer.is_some();
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError> {
        match &frame.data {
            RenderFrameData::Bgra32(data) => {
                self.upload_bgra(frame.width, frame.height, data)?;
            }
            RenderFrameData::Rgb24(data) => {
                let expected = frame
                    .width
                    .checked_mul(frame.height)
                    .and_then(|pixels| pixels.checked_mul(3))
                    .ok_or_else(|| RenderError::Message("Metal RGB frame size overflow".into()))?;
                if data.len() != expected {
                    return Err(RenderError::Message(format!(
                        "Metal RGB frame bytes mismatch: expected {expected}, got {}",
                        data.len()
                    )));
                }
                let output_len = frame.width * frame.height * 4;
                if self.scratch_bgra.len() != output_len {
                    self.scratch_bgra.resize(output_len, 0);
                }
                for (src, dst) in data
                    .chunks_exact(3)
                    .zip(self.scratch_bgra.chunks_exact_mut(4))
                {
                    dst[0] = src[2];
                    dst[1] = src[1];
                    dst[2] = src[0];
                    dst[3] = 255;
                }
                let bgra = std::mem::take(&mut self.scratch_bgra);
                self.upload_bgra(frame.width, frame.height, &bgra)?;
                self.scratch_bgra = bgra;
            }
            #[cfg(windows)]
            RenderFrameData::D3D11SharedNv12 { .. } | RenderFrameData::D3D11SharedP010 { .. } => {
                return Err(RenderError::Message(
                    "Metal renderer does not accept D3D11 shared textures".to_string(),
                ));
            }
        }

        self.uploaded_frame_count = self.uploaded_frame_count.saturating_add(1);
        self.last_width = frame.width;
        self.last_height = frame.height;
        self.last_pixel_format = Some(frame.pixel_format);
        self.present_if_attached(frame.width, frame.height);
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
    use super::*;

    #[test]
    fn descriptor_reports_metal_runtime() {
        let descriptor = MacosRendererFactory.descriptor();

        assert_eq!(descriptor.id, "metal");
        assert_eq!(descriptor.runtime_status, RuntimeStatus::RuntimeBacked);
        assert!(descriptor
            .supported_formats
            .contains(&RenderPixelFormat::Bgra32));
        assert!(descriptor
            .supported_formats
            .contains(&RenderPixelFormat::Rgb24));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn copy_region_size_clamps_to_drawable() {
        assert_eq!(copy_region_size(1920, 1080, 1280, 720), Some((1280, 720)));
        assert_eq!(copy_region_size(640, 360, 1280, 720), Some((640, 360)));
        assert_eq!(copy_region_size(0, 360, 1280, 720), None);
        assert_eq!(copy_region_size(640, 360, 1280, 0), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn scaled_drawable_dimension_never_returns_zero() {
        assert_eq!(scaled_drawable_dimension(640.0, 2.0), 1280);
        assert_eq!(scaled_drawable_dimension(0.0, 2.0), 1);
        assert_eq!(scaled_drawable_dimension(f64::NAN, 2.0), 1);
    }
}
