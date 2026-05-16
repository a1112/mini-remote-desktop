# OpenGL Renderer Design

## Goal

OpenGL is a portable native-render fallback for platforms or devices where the
preferred renderer is unavailable. It does not replace the Windows D3D11 shared
texture path, and it is not a zero-copy target in this phase.

## Scope

- Add a renderer crate exposed as `mrd-render-opengl`.
- Accept CPU-backed `Rgb24` and `Bgra32` frames.
- Reject D3D11 shared texture inputs with an explicit error.
- Wire `opengl` through the Rdesk harness and matrix selection.
- Keep the existing `render.opengl` WGL probe for independent visible-window
  smoke testing.

## Non-goals

- No D3D11 shared texture interop in OpenGL.
- No replacement of `DXGI -> NVENC -> NVDEC -> D3D11 shared/native render`.
- No shader-based NV12/P010 upload in this first step.
- No WebView frame transport changes.

## Capability Model

`render.opengl` is reported as a supported fallback. It can participate in
matrix runs only with `memory.cpu`.

`render.opengl + memory.d3d11_shared` is blocked and should suggest
`memory.cpu`, because the current OpenGL path has no D3D11 texture interop.

## Follow-up Path

1. Add real OpenGL texture upload and present on Windows via WGL.
2. Add EGL/GLX context creation on Linux.
3. Add NV12 shader conversion after CPU-backed BGRA is stable.
4. Consider platform-specific interop only after D3D11 parity work remains
   stable.
