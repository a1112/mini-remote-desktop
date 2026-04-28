use mrd_render::{
    d3d11_descriptor, BoxedRenderer, RenderError, RenderFrame, RenderPixelFormat, RenderTarget,
    RendererDescriptor, RendererFactory, RendererInstance, RendererSnapshot,
};

pub mod simd;

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
    width: u32,
    height: u32,
}

#[cfg(windows)]
struct SharedNv12Pipeline {
    vertex_shader: windows::Win32::Graphics::Direct3D11::ID3D11VertexShader,
    pixel_shader: windows::Win32::Graphics::Direct3D11::ID3D11PixelShader,
    sampler: windows::Win32::Graphics::Direct3D11::ID3D11SamplerState,
}

#[cfg(windows)]
const SHARED_NV12_VERTEX_SHADER: &str = r#"
struct VsOut {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

VsOut main(uint vertex_id : SV_VertexID) {
    float2 positions[3] = {
        float2(-1.0, -1.0),
        float2(-1.0,  3.0),
        float2( 3.0, -1.0)
    };
    float2 uvs[3] = {
        float2(0.0, 1.0),
        float2(0.0, -1.0),
        float2(2.0, 1.0)
    };

    VsOut output;
    output.position = float4(positions[vertex_id], 0.0, 1.0);
    output.uv = uvs[vertex_id];
    return output;
}
"#;

#[cfg(windows)]
const SHARED_NV12_PIXEL_SHADER: &str = r#"
Texture2D y_texture : register(t0);
Texture2D uv_texture : register(t1);
SamplerState linear_sampler : register(s0);

struct PsIn {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

float4 main(PsIn input) : SV_TARGET {
    float y = y_texture.Sample(linear_sampler, input.uv).r;
    float2 uv = uv_texture.Sample(linear_sampler, input.uv).rg - float2(0.5, 0.5);

    float r = y + 1.5748 * uv.y;
    float g = y - 0.1873 * uv.x - 0.4681 * uv.y;
    float b = y + 1.8556 * uv.x;
    return float4(saturate(float3(r, g, b)), 1.0);
}
"#;

pub struct D3d11Renderer {
    #[cfg(windows)]
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    #[cfg(windows)]
    context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    #[cfg(windows)]
    surface: Option<RenderSurface>,
    #[cfg(windows)]
    shared_nv12_pipeline: Option<SharedNv12Pipeline>,
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
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
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
                shared_nv12_pipeline: None,
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
            IDXGIDevice, IDXGIFactory2, IDXGISwapChain1, DXGI_SCALING_STRETCH,
            DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
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
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
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
        unsafe {
            self.device
                .CreateRenderTargetView(&back_buffer, None, Some(&mut render_target_view))
        }
        .map_err(|error| RenderError::Message(format!("创建 RTV 失败: {error}")))?;
        let render_target_view = render_target_view
            .ok_or_else(|| RenderError::Message("缺少 render target view".into()))?;

        Ok(Some(RenderSurface {
            swap_chain,
            back_buffer,
            render_target_view,
            width,
            height,
        }))
    }

    #[cfg(windows)]
    fn average_clear_color(frame: &RenderFrame) -> [f32; 4] {
        use mrd_render::RenderFrameData;
        let data = match &frame.data {
            RenderFrameData::Rgb24(data) => data,
            RenderFrameData::Bgra32(data) => data,
            #[cfg(windows)]
            RenderFrameData::D3D11SharedNv12 { .. } => {
                return [0.05, 0.05, 0.05, 1.0];
            }
        };

        if data.is_empty() {
            return [0.05, 0.05, 0.05, 1.0];
        }

        let mut r: u64 = 0;
        let mut g: u64 = 0;
        let mut b: u64 = 0;
        let mut pixels: u64 = 0;

        for chunk in data.chunks_exact(3) {
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
                .Present(0, 0)
                .ok()
                .map_err(|error| RenderError::Message(format!("present 失败: {error}")))?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn present_uploaded_frame_bgra(&self, frame: &RenderFrame) -> Result<(), RenderError> {
        use mrd_render::RenderFrameData;
        use windows::Win32::Graphics::Direct3D11::D3D11_BOX;
        let Some(surface) = self.surface.as_ref() else {
            return Ok(());
        };

        let data = match &frame.data {
            RenderFrameData::Bgra32(data) => data,
            _ => return Err(RenderError::Message("Expected Bgra32 frame data".into())),
        };

        let expected = frame
            .width
            .checked_mul(frame.height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| RenderError::Message("frame size overflow".into()))?;
        if data.len() != expected {
            return Err(RenderError::Message(format!(
                "Bgra32 frame bytes mismatch: expected {expected}, got {}",
                data.len()
            )));
        }

        let surface_width = surface.width as usize;
        let surface_height = surface.height as usize;
        let upload_data;
        let (data, upload_width, upload_height) =
            if frame.width == surface_width && frame.height == surface_height {
                (data.as_slice(), frame.width, frame.height)
            } else {
                upload_data = Self::scale_bgra_to_fit(
                    data,
                    frame.width,
                    frame.height,
                    surface_width,
                    surface_height,
                )?;
                (upload_data.as_slice(), surface_width, surface_height)
            };
        let row_pitch = upload_width
            .checked_mul(4)
            .ok_or_else(|| RenderError::Message("row pitch overflow".into()))?
            as u32;
        let copy_width = upload_width.min(surface_width);
        let copy_height = upload_height.min(surface_height);
        if copy_width == 0 || copy_height == 0 {
            return Ok(());
        }
        let copy_box = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: copy_width as u32,
            bottom: copy_height as u32,
            back: 1,
        };

        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(surface.render_target_view.clone())]), None);
            self.context
                .ClearRenderTargetView(&surface.render_target_view, &[0.0, 0.0, 0.0, 1.0]);
            self.context.UpdateSubresource(
                &surface.back_buffer,
                0,
                Some(&copy_box as *const D3D11_BOX),
                data.as_ptr() as *const core::ffi::c_void,
                row_pitch,
                0,
            );
            surface
                .swap_chain
                .Present(0, 0)
                .ok()
                .map_err(|error| RenderError::Message(format!("present 失败: {error}")))?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn rgb24_to_bgra(frame: &RenderFrame) -> Result<Vec<u8>, RenderError> {
        use mrd_render::RenderFrameData;
        let data = match &frame.data {
            RenderFrameData::Rgb24(data) => data,
            _ => return Err(RenderError::Message("Expected Rgb24 frame data".into())),
        };

        let expected = frame
            .width
            .checked_mul(frame.height)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or_else(|| RenderError::Message("frame size overflow".into()))?;
        if data.len() != expected {
            return Err(RenderError::Message(format!(
                "Rgb24 frame bytes mismatch: expected {expected}, got {}",
                data.len()
            )));
        }

        let mut bgra = vec![0_u8; frame.width * frame.height * 4];
        simd::rgb24_to_bgra(data, &mut bgra, frame.width, frame.height);
        Ok(bgra)
    }

    #[cfg(windows)]
    fn scale_bgra_to_fit(
        source: &[u8],
        source_width: usize,
        source_height: usize,
        target_width: usize,
        target_height: usize,
    ) -> Result<Vec<u8>, RenderError> {
        if source_width == 0 || source_height == 0 || target_width == 0 || target_height == 0 {
            return Ok(Vec::new());
        }

        let source_len = source_width
            .checked_mul(source_height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| RenderError::Message("source frame size overflow".into()))?;
        if source.len() != source_len {
            return Err(RenderError::Message(format!(
                "BGRA source bytes mismatch: expected {source_len}, got {}",
                source.len()
            )));
        }

        let target_len = target_width
            .checked_mul(target_height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| RenderError::Message("target frame size overflow".into()))?;
        let mut target = vec![0_u8; target_len];

        let width_limited_height =
            ((target_width as u128 * source_height as u128) / source_width as u128) as usize;
        let (draw_width, draw_height) = if width_limited_height <= target_height {
            (target_width, width_limited_height.max(1))
        } else {
            let height_limited_width =
                ((target_height as u128 * source_width as u128) / source_height as u128) as usize;
            (height_limited_width.max(1), target_height)
        };
        let offset_x = (target_width - draw_width) / 2;
        let offset_y = (target_height - draw_height) / 2;

        for y in 0..draw_height {
            let source_y = (y * source_height / draw_height).min(source_height - 1);
            for x in 0..draw_width {
                let source_x = (x * source_width / draw_width).min(source_width - 1);
                let source_idx = (source_y * source_width + source_x) * 4;
                let target_idx = ((offset_y + y) * target_width + offset_x + x) * 4;
                target[target_idx..target_idx + 4]
                    .copy_from_slice(&source[source_idx..source_idx + 4]);
            }
        }

        Ok(target)
    }

    #[cfg(windows)]
    fn present_uploaded_frame(&self, frame: &RenderFrame) -> Result<(), RenderError> {
        use windows::Win32::Graphics::Direct3D11::D3D11_BOX;
        let Some(surface) = self.surface.as_ref() else {
            return Ok(());
        };

        let bgra = Self::rgb24_to_bgra(frame)?;
        let surface_width = surface.width as usize;
        let surface_height = surface.height as usize;
        let upload_data;
        let (data, upload_width, upload_height) =
            if frame.width == surface_width && frame.height == surface_height {
                (bgra.as_slice(), frame.width, frame.height)
            } else {
                upload_data = Self::scale_bgra_to_fit(
                    &bgra,
                    frame.width,
                    frame.height,
                    surface_width,
                    surface_height,
                )?;
                (upload_data.as_slice(), surface_width, surface_height)
            };
        let row_pitch = upload_width
            .checked_mul(4)
            .ok_or_else(|| RenderError::Message("row pitch overflow".into()))?
            as u32;
        let copy_width = upload_width.min(surface_width);
        let copy_height = upload_height.min(surface_height);
        if copy_width == 0 || copy_height == 0 {
            return Ok(());
        }
        let copy_box = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: copy_width as u32,
            bottom: copy_height as u32,
            back: 1,
        };

        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(surface.render_target_view.clone())]), None);
            self.context
                .ClearRenderTargetView(&surface.render_target_view, &[0.0, 0.0, 0.0, 1.0]);
            self.context.UpdateSubresource(
                &surface.back_buffer,
                0,
                Some(&copy_box as *const D3D11_BOX),
                data.as_ptr() as *const core::ffi::c_void,
                row_pitch,
                0,
            );
            surface
                .swap_chain
                .Present(0, 0)
                .ok()
                .map_err(|error| RenderError::Message(format!("present 失败: {error}")))?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn compile_shader(source: &str, target: &'static [u8]) -> Result<Vec<u8>, RenderError> {
        use windows::core::PCSTR;
        use windows::Win32::Graphics::Direct3D::{Fxc::D3DCompile, ID3DBlob, ID3DInclude};

        let mut code = None::<ID3DBlob>;
        let mut errors = None::<ID3DBlob>;
        let result = unsafe {
            D3DCompile(
                source.as_ptr() as *const core::ffi::c_void,
                source.len(),
                PCSTR::null(),
                None,
                None::<&ID3DInclude>,
                PCSTR(b"main\0".as_ptr()),
                PCSTR(target.as_ptr()),
                0,
                0,
                &mut code,
                Some(&mut errors),
            )
        };

        if let Err(error) = result {
            let details = errors
                .as_ref()
                .map(|blob| unsafe {
                    let bytes = core::slice::from_raw_parts(
                        blob.GetBufferPointer() as *const u8,
                        blob.GetBufferSize(),
                    );
                    String::from_utf8_lossy(bytes).trim().to_string()
                })
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| error.to_string());
            return Err(RenderError::Message(format!(
                "compile D3D11 shared NV12 shader failed: {details}"
            )));
        }

        let code = code.ok_or_else(|| RenderError::Message("missing shader bytecode".into()))?;
        let bytes = unsafe {
            core::slice::from_raw_parts(code.GetBufferPointer() as *const u8, code.GetBufferSize())
        };
        Ok(bytes.to_vec())
    }

    #[cfg(windows)]
    fn create_shared_nv12_pipeline(&self) -> Result<SharedNv12Pipeline, RenderError> {
        use windows::Win32::Graphics::Direct3D11::{
            ID3D11ClassLinkage, ID3D11PixelShader, ID3D11SamplerState, ID3D11VertexShader,
            D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_SAMPLER_DESC,
            D3D11_TEXTURE_ADDRESS_CLAMP,
        };

        let vertex_code = Self::compile_shader(SHARED_NV12_VERTEX_SHADER, b"vs_5_0\0")?;
        let pixel_code = Self::compile_shader(SHARED_NV12_PIXEL_SHADER, b"ps_5_0\0")?;

        let mut vertex_shader = None::<ID3D11VertexShader>;
        let mut pixel_shader = None::<ID3D11PixelShader>;
        unsafe {
            self.device
                .CreateVertexShader(
                    &vertex_code,
                    None::<&ID3D11ClassLinkage>,
                    Some(&mut vertex_shader),
                )
                .map_err(|error| {
                    RenderError::Message(format!(
                        "create shared NV12 vertex shader failed: {error}"
                    ))
                })?;
            self.device
                .CreatePixelShader(
                    &pixel_code,
                    None::<&ID3D11ClassLinkage>,
                    Some(&mut pixel_shader),
                )
                .map_err(|error| {
                    RenderError::Message(format!("create shared NV12 pixel shader failed: {error}"))
                })?;
        }

        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0, 0.0, 0.0, 0.0],
            MinLOD: 0.0,
            MaxLOD: f32::MAX,
        };
        let mut sampler = None::<ID3D11SamplerState>;
        unsafe {
            self.device
                .CreateSamplerState(&sampler_desc, Some(&mut sampler))
                .map_err(|error| {
                    RenderError::Message(format!("create shared NV12 sampler failed: {error}"))
                })?;
        }

        Ok(SharedNv12Pipeline {
            vertex_shader: vertex_shader
                .ok_or_else(|| RenderError::Message("missing vertex shader".into()))?,
            pixel_shader: pixel_shader
                .ok_or_else(|| RenderError::Message("missing pixel shader".into()))?,
            sampler: sampler.ok_or_else(|| RenderError::Message("missing sampler".into()))?,
        })
    }

    #[cfg(windows)]
    fn ensure_shared_nv12_pipeline(&mut self) -> Result<&SharedNv12Pipeline, RenderError> {
        if self.shared_nv12_pipeline.is_none() {
            self.shared_nv12_pipeline = Some(self.create_shared_nv12_pipeline()?);
        }
        Ok(self.shared_nv12_pipeline.as_ref().unwrap())
    }

    #[cfg(windows)]
    fn open_shared_texture_srv(
        &self,
        shared_handle: isize,
    ) -> Result<windows::Win32::Graphics::Direct3D11::ID3D11ShaderResourceView, RenderError> {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Graphics::Direct3D11::{
            ID3D11Resource, ID3D11ShaderResourceView, ID3D11Texture2D,
        };

        if shared_handle == 0 {
            return Err(RenderError::Message("shared texture handle is zero".into()));
        }

        let mut texture = None::<ID3D11Texture2D>;
        unsafe {
            self.device
                .OpenSharedResource(HANDLE(shared_handle), &mut texture)
                .map_err(|error| {
                    RenderError::Message(format!("open shared D3D11 texture failed: {error}"))
                })?;
        }
        let texture =
            texture.ok_or_else(|| RenderError::Message("missing shared texture".into()))?;
        let resource: ID3D11Resource = texture.cast().map_err(|error| {
            RenderError::Message(format!("cast shared texture to resource failed: {error}"))
        })?;

        let mut srv = None::<ID3D11ShaderResourceView>;
        unsafe {
            self.device
                .CreateShaderResourceView(&resource, None, Some(&mut srv))
                .map_err(|error| {
                    RenderError::Message(format!("create shared texture SRV failed: {error}"))
                })?;
        }
        srv.ok_or_else(|| RenderError::Message("missing shared texture SRV".into()))
    }

    #[cfg(windows)]
    fn present_shared_texture_frame(&mut self, frame: &RenderFrame) -> Result<(), RenderError> {
        use mrd_render::RenderFrameData;
        use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
        use windows::Win32::Graphics::Direct3D11::{ID3D11ShaderResourceView, D3D11_VIEWPORT};

        let Some(surface) = self.surface.as_ref() else {
            return Ok(());
        };

        let (shared_handle_y, shared_handle_uv) = match &frame.data {
            RenderFrameData::D3D11SharedNv12 {
                shared_handle_y,
                shared_handle_uv,
                width: _,
                height: _,
            } => (*shared_handle_y, *shared_handle_uv),
            _ => {
                return Err(RenderError::Message(
                    "Expected D3D11SharedNv12 frame data".into(),
                ))
            }
        };

        let surface_width = surface.width;
        let surface_height = surface.height;
        let render_target_view = surface.render_target_view.clone();
        let swap_chain = surface.swap_chain.clone();
        let y_srv = self.open_shared_texture_srv(shared_handle_y)?;
        let uv_srv = self.open_shared_texture_srv(shared_handle_uv)?;
        let (vertex_shader, pixel_shader, sampler) = {
            let pipeline = self.ensure_shared_nv12_pipeline()?;
            (
                pipeline.vertex_shader.clone(),
                pipeline.pixel_shader.clone(),
                pipeline.sampler.clone(),
            )
        };

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: surface_width as f32,
            Height: surface_height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let srvs = [Some(y_srv), Some(uv_srv)];
        let samplers = [Some(sampler)];
        let empty_srvs: [Option<ID3D11ShaderResourceView>; 2] = [None, None];

        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(render_target_view)]), None);
            self.context.RSSetViewports(Some(&[viewport]));
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&vertex_shader, None);
            self.context.PSSetShader(&pixel_shader, None);
            self.context.PSSetSamplers(0, Some(&samplers));
            self.context.PSSetShaderResources(0, Some(&srvs));
            self.context.Draw(3, 0);
            self.context.PSSetShaderResources(0, Some(&empty_srvs));
            swap_chain
                .Present(0, 0)
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
                RenderTarget::WindowHandle(window_handle) => {
                    self.attach_window_surface(window_handle)?
                }
            };
        }

        self.attached_to_target = true;
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError> {
        use mrd_render::RenderFrameData;
        match &frame.data {
            RenderFrameData::Rgb24(_) =>
            {
                #[cfg(windows)]
                if self.surface.is_some() {
                    self.present_uploaded_frame(&frame)?;
                } else {
                    self.present_clear_frame(&frame)?;
                }
            }
            RenderFrameData::Bgra32(_) =>
            {
                #[cfg(windows)]
                if self.surface.is_some() {
                    self.present_uploaded_frame_bgra(&frame)?;
                } else {
                    self.present_clear_frame(&frame)?;
                }
            }
            #[cfg(windows)]
            RenderFrameData::D3D11SharedNv12 { .. } =>
            {
                #[cfg(windows)]
                if self.surface.is_some() {
                    self.present_shared_texture_frame(&frame)?;
                } else {
                    self.present_clear_frame(&frame)?;
                }
            }
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
            .upload_frame(RenderFrame::from_rgb24(16, 16, vec![128; 16 * 16 * 3]))
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
