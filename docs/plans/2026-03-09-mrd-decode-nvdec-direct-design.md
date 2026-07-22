# Direct NVDEC Decoder Design

**Date:** 2026-03-09

**Goal:** Replace the current probe-only `nvdec` backend with a unified Windows-only direct NVDEC decoder path that can create a real decoder, accept H264 Annex-B access units, and eventually emit CPU `Rgb24` frames through the existing `VideoDecoder` interface.

## Context

`mrd-decode-nvdec` already has a Windows runtime probe that dynamically loads `nvcuda.dll` and `nvcuvid.dll` and verifies a small set of core exports. That established a stable entry point without relying on the broken `cuda-rs` / `nvcodec` / `npp` dependency chain.

The next step is to keep that direct Windows route and extend it into a real decoder implementation. The existing public contract stays unchanged:

- `push_access_unit(&[u8]) -> Result<(), DecoderError>`
- `drain_decoded_frames() -> Vec<DecodedFrame>`

`mrd-decode` remains the factory and registry layer. `mrd-decode-nvdec` owns all Windows/NVDEC-specific logic.

## Chosen Approach

Use a single unified path for both tests and production:

- `mrd-decode::create_decoder("nvdec")` returns a real `NvdecDecoder`
- crate-level tests and integration tests use the same implementation
- there is no separate test-only feeding path

Implementation still proceeds in phases, but every phase extends the same concrete decoder type instead of building placeholders beside it.

## Alternatives Considered

### 1. Direct native NVDEC FFI with minimal hand-written bindings

This is the chosen approach.

Pros:

- avoids broken third-party CUDA/NVDEC wrapper crates
- keeps dependency surface small and Windows-specific
- matches the already working runtime probe architecture
- gives full control over error reporting and integration boundaries

Cons:

- requires careful FFI and callback handling
- more manual work than using an existing wrapper

### 2. Direct NVDEC with FFmpeg as the parser/demux layer

Not chosen for this phase.

Pros:

- stronger bitstream parsing support
- easier long-term codec expansion

Cons:

- reintroduces FFmpeg build and packaging complexity
- broadens the moving parts before NVDEC session creation is proven

### 3. Fake NVDEC backend with OpenH264 decode under the hood

Rejected.

Pros:

- easy to make tests pass

Cons:

- does not prove the real NVDEC route
- creates misleading behavior and future rewrite cost

## Architecture

The implementation will be split into three internal layers inside `mrd-decode-nvdec`.

### API Layer

`CudaApi` and `NvcuvidApi` dynamically resolve driver and NVDEC functions through `LoadLibraryA` and `GetProcAddress`.

This layer owns:

- DLL loading
- symbol resolution
- typed function pointers
- low-level error names and API call wrappers

### Session Layer

`NvdecSession` owns the NVDEC runtime objects and callback state:

- CUDA initialization
- CUDA device selection
- CUDA context creation
- NVDEC parser creation
- NVDEC decoder creation
- parser callback bookkeeping

This layer bridges the C callback model into Rust-owned state via opaque pointers and explicit lifetime control.

### Decoder Layer

`NvdecDecoder` implements `mrd_decode::VideoDecoder` and is the only type the rest of the workspace should care about.

Responsibilities:

- validate input access units
- feed Annex-B H264 access units into the parser
- collect decoded output
- convert decoded surfaces into CPU `Rgb24`
- expose pending frames through `drain_decoded_frames()`

## Data Flow

1. `create_decoder("nvdec")` constructs `NvdecDecoder::new()`.
2. `NvdecDecoder::new()` creates the API layer and session layer, initializes CUDA, and prepares the parser/decoder.
3. `push_access_unit()` validates that the input is an H264 Annex-B access unit and passes it into `cuvidParseVideoData`.
4. Parser callbacks discover or validate stream format and trigger decode.
5. Display/decode callbacks map the NVDEC output surface into CPU-visible memory.
6. The mapped frame is converted into `Rgb24` and pushed into `pending_frames`.
7. `drain_decoded_frames()` returns those accumulated frames.

## Scope Limits

This phase only targets:

- Windows
- H264
- 8-bit surfaces
- single decoder instance per session
- fixed-resolution stream assumption
- CPU `Rgb24` output

This phase explicitly does not include:

- HEVC
- dynamic resolution change handling
- GPU-native frame output
- multi-stream/shared CUDA context orchestration
- optimized color conversion

## Error Handling

Errors must remain explicit and actionable.

### Environment Errors

Examples:

- `nvcuda.dll` missing
- `nvcuvid.dll` missing
- required symbols missing
- no CUDA device
- CUDA context creation failed
- NVDEC parser or decoder creation failed

These must include the concrete API name or runtime element in the message.

### Input Errors

Examples:

- non-Annex-B input
- unsupported codec flavor
- incomplete SPS/PPS state
- unsupported stream parameters

These should be returned from `push_access_unit()` as structured decoder errors.

### Runtime Decode Errors

Examples:

- parse failure
- decode failure
- map/unmap failure
- conversion failure

These must fail the current call clearly and leave the decoder in a defined state.

## Testing Strategy

The same decoder path will be exercised incrementally.

### Stage 1

Add a test that constructs `NvdecDecoder::new()` and expects either:

- success on supported Windows/NVIDIA hosts, or
- a clear unsupported/runtime error

### Stage 2

Add a test that feeds one valid H264 Annex-B access unit and expects:

- no panic
- no malformed-input error for a valid AU
- evidence that real NVDEC parser/decode activity occurred

### Stage 3

Add a test that feeds a valid H264 AU and expects:

- at least one decoded frame
- `PixelFormat::Rgb24`
- expected dimensions for the chosen sample

### Integration

Once crate-level tests pass, wire `mrd-decode::create_decoder("nvdec")` to return the real decoder and run the existing decode and app regression tests.

## Success Criteria

This design is complete when:

- `mrd-decode-nvdec` contains a real `NvdecDecoder`
- `create_decoder("nvdec")` returns a decoder instead of a placeholder error when supported
- valid H264 Annex-B input reaches the real NVDEC path
- decoded frames are surfaced as CPU `Rgb24`
- existing app tests remain green

## Current Status

- `mrd-decode-nvdec` now constructs a real Windows-only NVDEC session directly from `nvcuda.dll` and `nvcuvid.dll`.
- `NvdecDecoder::new()` creates the CUDA context and NVDEC parser.
- `push_access_unit()` feeds real H264 Annex-B access units into `cuvidParseVideoData()`.
- display callbacks map NVDEC output surfaces, copy NV12 data back to CPU memory, and convert to `Rgb24`.
- `mrd-decode::create_decoder("nvdec")` now returns a real decoder adapter instead of a placeholder error.
- Verified with:
  - `cargo test -p mrd-decode-nvdec -- --nocapture`
  - `cargo test -p mrd-decode nvdec -- --nocapture`
  - `cargo test -p app -- --nocapture`
- Practical note: the tiny `16x16` sample used earlier was too small for reliable NVDEC decoder creation on this host, so the decode-path tests now use a `128x128` H264 sample.
