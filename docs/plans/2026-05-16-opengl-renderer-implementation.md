# OpenGL Renderer Implementation Plan

## Implementation Steps

1. Add `crates/mrd-render-opengl`.
2. Export `OpenglRendererFactory` and `OpenglRenderer`.
3. Keep the renderer CPU-backed:
   - `Rgb24` length must equal `width * height * 3`.
   - `Bgra32` length must equal `width * height * 4`.
   - shared texture formats fail fast.
4. Wire `RendererType::Opengl` into the Rdesk test harness.
5. Allow custom/matrix harness runs for OpenGL when `zero_copy != true`.
6. Add `render.opengl` to service and frontend capability models.
7. Add matrix UI option and validation.

## Tests

- `mrd-render-opengl` descriptor and upload validation.
- Harness config maps `renderer_type: "opengl"` only when render display is
  enabled.
- Platform validation allows OpenGL CPU-memory runs and rejects OpenGL with
  D3D11 shared memory.
- Frontend capability evaluation blocks `opengl + d3d11_shared`.
- Matrix UI exposes OpenGL and disables D3D11 shared memory when OpenGL is the
  selected renderer.

## Acceptance

- `cargo test -p mrd-render-opengl`
- `cargo test -p app opengl -- --nocapture`
- `pnpm test -- --run src/app/services/capabilityMatrix.test.ts src/app/components/TestWorkbench/MatrixTestPage.test.tsx`
- `pnpm type-check`
- `cargo build -p app -p mrd-service -p mrd-render-opengl`
