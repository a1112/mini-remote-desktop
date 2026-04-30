#![allow(unexpected_cfgs)]

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
    texture: Option<metal::Texture>,
    texture_width: usize,
    texture_height: usize,
    scratch_bgra: Vec<u8>,
    attached_to_target: bool,
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
            texture: None,
            texture_width: 0,
            texture_height: 0,
            scratch_bgra: Vec::new(),
            attached_to_target: false,
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
        use objc::{
            msg_send,
            runtime::{Object, YES},
            sel, sel_impl,
        };

        if ns_view == 0 {
            return Err(RenderError::Message(
                "Metal renderer requires a non-null NSView render target".to_string(),
            ));
        }

        let mut layer = metal::MetalLayer::new();
        layer.set_device(&self.device);
        layer.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        layer.set_presents_with_transaction(false);
        layer.set_framebuffer_only(false);
        layer.set_display_sync_enabled(true);
        layer.set_masks_to_bounds(true);
        layer.set_opaque(true);
        layer.remove_all_animations();

        let layer_object = layer.as_mut() as *mut _ as *mut Object as usize;
        run_on_main_thread_sync(move || unsafe {
            let view = ns_view as *mut Object;
            if view.is_null() {
                return Err("macOS render target NSView pointer is null".to_string());
            }
            let _: () = msg_send![view, setWantsLayer: YES];
            let layer_object = layer_object as *mut Object;
            let bounds: core_graphics_types::geometry::CGRect = msg_send![view, bounds];
            let window: *mut Object = msg_send![view, window];
            let contents_scale = if window.is_null() {
                1.0
            } else {
                msg_send![window, backingScaleFactor]
            };
            let _: () = msg_send![layer_object, setFrame: bounds];
            let _: () = msg_send![layer_object, setContentsScale: contents_scale];
            let _: () = msg_send![layer_object, setNeedsDisplayOnBoundsChange: YES];
            let _: () = msg_send![layer_object, setZPosition: 1000.0_f64];
            let _: () = msg_send![view, setLayer: layer_object];
            let _: () = msg_send![view, setNeedsDisplay: YES];
            Ok(())
        })
        .map_err(RenderError::Message)?;

        self.layer = Some(layer);
        Ok(())
    }

    fn present_if_attached(&mut self, width: usize, height: usize) {
        objc::rc::autoreleasepool(|| self.present_if_attached_inner(width, height));
    }

    fn present_if_attached_inner(&mut self, width: usize, height: usize) {
        let Some(layer) = self.layer.as_ref() else {
            return;
        };
        let Some(texture) = self.texture.as_ref() else {
            return;
        };

        layer.set_drawable_size(core_graphics_types::geometry::CGSize::new(
            width as f64,
            height as f64,
        ));
        let Some(drawable) = layer.next_drawable() else {
            return;
        };

        let command_buffer = self.command_queue.new_command_buffer();
        let blit = command_buffer.new_blit_command_encoder();
        blit.copy_from_texture(
            texture,
            0,
            0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
            metal::MTLSize::new(width as u64, height as u64, 1),
            drawable.texture(),
            0,
            0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
        );
        blit.end_encoding();
        command_buffer.present_drawable(drawable);
        command_buffer.commit();
        command_buffer.wait_until_scheduled();
    }
}

#[cfg(target_os = "macos")]
fn run_on_main_thread_sync<F>(f: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String> + Send,
{
    if unsafe { pthread_main_np() } != 0 {
        return f();
    }

    let mut result = None;
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
            RenderFrameData::D3D11SharedNv12 { .. } => {
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
}
