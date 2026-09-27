# Decode → render zero CPU copy audit and implementation

Date: 2026-09-27. Scope: active Rust decoder/render crates; historical `junk/` implementations were not used.

## Meaning of the path

The NVDEC shared path is **zero CPU pixel readback**, not zero physical copies. It performs two CUDA device-to-array copies (Y and UV) into shader-readable D3D11 textures, then samples those planes into the swap-chain render target. The CPU fallback deliberately downloads NV12/P010; D3D11 now uploads those original planes directly, avoiding a full-frame CPU RGB conversion and intermediate RGB allocation.

Compressed packet copies and capture/encode are separate accounting boundaries. An initialized renderer, an advertised format, a `WindowHandle(0)` upload, and a hardware test that returns early because DLLs are absent do not establish a working shared decode/render path.

## Changes

| Component | Change | Reason |
| --- | --- | --- |
| `mrd-decode-nvdec` | DLL/codec/stage/OS-error diagnostics; capability query under an active CUDA context | Separate missing runtime, unsupported codec, and CUDA interop failures |
| `mrd-decode-nvdec` | Shared resource initialization in sequence callback before first display | Avoid first-frame CPU readback |
| `mrd-decode-nvdec` | `require_shared_texture()` and explicit error before DtoH | A required shared decoder must never hide failure by downloading then discarding pixels |
| `mrd-decode-nvdec` | Four shared plane slots with leases and cached CUDA registrations | Preserve queued/in-flight frame contents; reuse registrations and one matching D3D11 device |
| `mrd-pipeline-core`, `mrd-render` | `GpuFrameLease`, optional shared-frame lease, `with_gpu_lease` | Forward resource ownership through decoder/application/render boundaries |
| `mrd-render-d3d11` | EVENT-query retirement worker with multithread-protected context | Retain producer storage until GPU sampling completes, including when no next frame arrives |
| `mrd-render-d3d11` | CPU NV12/P010 plane upload cache | Reuse R8/RG8 or R16/RG16 textures; retain original pitch and 16-bit words |
| `mrd-render-d3d11` | One adapter-selection pass on first shared import; cache limit four | Align renderer to actual shareable resource and bound historical-resolution imports |
| `mrd-decode` | NV12/P010 descriptors corrected; AV1 shared factory | Make candidate capabilities match actual outputs |
| `mrd-render-opengl` | Explicit rejection of leased GPU frames | Existing OpenGL path has no validated GPU-completion lease integration |
| `mrd-session-agent/windows_render` | Runtime keyframe fallback inside the authorized render worker; live backend metrics | A constructor-success/first-frame interop failure must not terminate rendering without trying CPU NVDEC/software |

New factory ID: `nvdec_av1_d3d11_shared`. CPU P010 render constructor: `RenderFrame::from_p010(width, height, data, pitch)`. Shared frame constructors retain their argument lists; attach ownership with `.with_gpu_lease(lease)`. Direct enum matches must forward `lease`.

The Session Agent's existing route/resource/session checks remain authoritative. `RenderDecoderFactory::create_fallback` defaults to no fallback for existing custom factories. Production runtime recovery retries the same failed keyframe through CPU NVDEC then software, at most twice. A successful retry with no output is valid buffering and retains that decoder. Non-keyframe failures and exhausted fallbacks stay fail-closed; metrics report the successfully selected backend.

## Lifetime and error-path review

* A slot is reusable only when its texture owner's `Arc` count is one (the pool's reference). Renderer SRV caches hold COM references, never the producer lease; caching therefore cannot permanently reserve all decoder slots.
* The pool contains at most four texture pairs across both old and new dimensions/formats. Resize replaces only an unleased slot. Old frames retain the original COM textures even if the decoder/pool is destroyed.
* When all four slots are leased, the display callback drops that output before mapping/downloading and increments `shared_pool_backpressure_frames`. It returns success, so normal backpressure does not activate software fallback or request another keyframe.
* CUDA registration is cached per slot, restored to its slot on map/copy failure, unregistered on replacement/drop under the decoder context. Each slot tracks mapped and poisoned state. Any graphics unmap/unregister failure quarantines the slot and preserves registration plus COM ownership; neither the next frame nor resize can reuse/replace it. Cleanup attempts during drop preserve failures until CUDA context destruction. The optional path switches to CPU; strict output reports an error without DtoH. A successful shared frame is published only after CUDA graphics unmap succeeds.
* The D3D11 renderer creates the completion query before issuing draw commands. After sampling, it ends the query, flushes, and submits a cloned lease before `Present`. Thus a failed/nonblocking `Present` still retains input storage until sampling finishes.
* At most eight completion records are admitted per renderer. One render thread submits; the worker only retires. Queue locks are held only for push/pop/wakeup, never across GPU polling. Immediate-context access is serialized with `ID3D11Multithread::SetMultithreadProtected(true)`.
* Retirement is independent of future frames. `Drop` signals shutdown without joining a potentially stalled GPU. The worker keeps device/context/query/lease ownership until completion or device removal.
* After two seconds without completion, the renderer stops admission and reports an explicit timeout error. A permanently stalled device whose driver never reports removal can retain this bounded queue and one worker until process exit; freeing those leases on a timer would risk overwriting live GPU inputs. This is a per-renderer bound, not a claim that arbitrary repeated creation of stalled renderer instances has a global cap.
* Adapter switching clears surface, shader/import caches, CPU plane cache, and the active retirement tracker. Outstanding old-device queries retain their own context and leases. Later incompatible/invalid imports fail clearly. Legacy `IDXGIResource::GetSharedHandle` handles cannot be passed to the NT-handle-only `GetSharedResourceAdapterLuid`, so a bounded adapter-open search is used once.

## Hardware findings

The normal process loader cannot find `nvcuda.dll`/`nvcuvid.dll` on this host. A workspace-local runtime directory contains aliases copied from the already installed NVIDIA driver, without system PATH/driver changes. With that directory prepended only to the test process PATH, CUDA initialization and H.264 CPU-output decoding run on the GPU.

CUDA → D3D11 registration still returns `CUDA_ERROR_INVALID_DEVICE` (101). Adapter matching alone does not fix it: independent probes confirm the active CUDA device and actual D3D11 device have the same LUID. Both custom and primary contexts synchronize successfully. R8/RG8/BGRA formats, plain/shared/keyed-NT textures, texture/base resource interfaces, and native C++ official-header calls reproduce the registration failure; `cuD3D11GetDevices` returns 999. See the companion transport capability audit and artifacts `artifacts/local-zero-copy/cuda-d3d11-probe.json` / native probe evidence.

The shared capability must therefore remain unverified/disabled on this installation. The strict test intentionally requires real shared output and must remain failing when registration fails. A passing error/fallback regression is evidence of correct failure handling, not evidence that CUDA shared rendering works.

## Verification

Initial regression failures were observed for missing DLL diagnostics, missing AV1 shared descriptor/incorrect output formats, and D3D11 rejection of CPU NV12. The corresponding focused checks passed after implementation.

Final scoped verification:

| Check | Result | What it establishes |
| --- | --- | --- |
| NVDEC crate lib tests | 26 passed, 1 ignored | Diagnostics, format/reconfigure decisions, fake-CUDA strict no-readback, real D3D11 storage pool/lease lifetime; two cleanup failure regressions first failed, then passed |
| Pipeline core lib tests | 18 passed | Core regressions |
| Render types lib tests | 4 passed | Render data/descriptor regressions |
| D3D11 renderer lib tests | 25 passed | Padded NV12, exact P010 GPU upload/readback, real shared R16 contents/adapter import/cache reuse, completion lease retirement and admission bound |
| OpenGL lib tests | 11 passed | Existing renderer regressions; this does not certify leased OpenGL GPU rendering |
| `mrd-decode --test nvdec` | 6 passed | Actual planar format descriptors and AV1 shared factory entry; conditional hardware tests in this command do not certify hardware |
| Session Agent Windows render tests | 10 passed | Includes 3 new runtime fallback regressions; source-extracted real worker showed 3 failures before the fix, then formal Cargo-built binary passed all 10 |
| NVDEC hardware probe suite with workspace runtime aliases | 14 passed, **1 failed** | Real CPU H.264 decode/thread handoff/resize/fallback passed. Required shared first frame failed at register code 101, with zero CPU readback |

The optional shared hardware probe took the explicit CPU fallback branch: `gpu_copy_bytes=0`, `cpu_readback_bytes=98304`, with source luma gradient retained. This is positive fallback evidence and negative shared-path evidence. The entire hardware suite ran for 3.59 seconds; most elapsed command time was waiting for the shared Cargo build lock.

The final combined Cargo lib run, after cleanup quarantine and adapter-binding cleanup, passed **84 tests, 1 ignored**. `git diff --check` passed for the touched source files. The Session Agent's 10 tests and decoder descriptor suite's 6 tests were verified separately. No long GPU performance tests were run by this subtask.

An independent final source review rechecked mapped/poisoned state transfer, every post-map error exit, partial registration failure, resize cleanup, and destruction ordering. It confirmed the cleanup reuse/owner-loss finding was closed, with no further defect found in those paths.

Relevant commands (PowerShell):

```powershell
cargo check -p mrd-decode-nvdec -p mrd-decode -p mrd-render-d3d11 -p mrd-render-opengl
cargo test -p mrd-pipeline-core -p mrd-render -p mrd-decode-nvdec -p mrd-render-d3d11 -p mrd-render-opengl --lib -- --test-threads=1
cargo test -p mrd-decode --test nvdec -- --test-threads=1
$env:PATH = (Join-Path (Get-Location) 'artifacts/local-zero-copy/2026-09-27/runtime') + ';' + $env:PATH
cargo test -p mrd-decode-nvdec --test nvdec_probe -- --nocapture --test-threads=1
```

The CPU-only descriptor/lease tests and tests with fake CUDA functions are separate from actual GPU decode. D3D11 upload tests intentionally perform staging readback to verify exact test pixels; this readback is test-only. Synthetic shared import tests verify adapter selection, shared resource contents, and SRV reuse without depending on CUDA registration. They do not claim a successful end-to-end NVDEC shared path.

## Remaining limits

Full GPU shared decode/render cannot be certified until the host interop failure is resolved and strict first-frame, sustained content, resize, and Main10/AV1 probes succeed. The current shader uses its existing fixed YUV conversion; color-range/matrix/HDR metadata propagation is outside this change. NVDEC odd display dimensions still rely on its existing 4:2:0 surface conventions, while CPU upload validation explicitly handles ceil-sized chroma planes. macOS CPU P010 rendering and OpenGL leased GPU-frame retirement remain explicitly unsupported.
