# DX11 H.264 Display Selection Optimization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add explicit monitor selection for the DX11 + H.264 local paths, with service-owned local dual-process LAN/QUIC/native as the performance baseline and WebRTC/WebCodecs browser preview as diagnostic only.

**Architecture:** Keep B on the existing `mrd-service` capture selection path: enumerate local Windows displays from `mrd-service`, pass selected `windows:display-shared:N` into LAN baseline runs, and preserve DXGI shared texture capture through `DxgiSharedTextureCapture::new_for_device_name`. Add A as a diagnostic source-id contract for browser WebRTC/WebCodecs preview so it does not silently fall back to primary display. The Rdesk shell only selects and forwards source ids; capture ownership stays in `mrd-service` or the existing local harness.

**Tech Stack:** Rust workspace (`mrd-ipc`, `mrd-service`, `app` Tauri backend, DXGI/NVENC crates), React/TypeScript/Vitest in `apps/Rdesk`, PowerShell benchmark canaries, Windows DXGI + NVENC H.264.

---

## Ground Rules

- Use @superpowers:test-driven-development for every implementation task: write a failing test first, run it, implement the minimal code, run the passing test.
- Do not use `junk/`.
- Prefer existing source ids. For Windows displays the primary selectable id is `windows:display-shared:N`; `windows:display:N` may be accepted as a display reference, but DX11/H.264 preview should still open DXGI shared capture for that display.
- B is the baseline. Do not make WebRTC/WebCodecs browser preview the performance claim path.
- Commit after each task that passes its focused tests.

## Current Facts

- `apps/mrd-service/src/capture_source.rs` already enumerates Windows display sources and prefers `display_shared`.
- `apps/mrd-service/src/lan_discovery.rs` already maps selected `windows:display-shared:*` to `DxgiSharedTextureCapture::new_for_device_name(...)` for LAN sender capture.
- `apps/mrd-service/src/browser_webrtc_preview.rs` and `apps/mrd-service/src/browser_webcodecs_preview.rs` still use `DxgiSharedTextureCapture::new_primary()`.
- `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx` hides capture source selection during local preview.
- `tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1` already accepts `-CaptureSourceId`; use this for B acceptance.

---

### Task 1: IPC Contract For Local Capture Source Listing

**Files:**
- Modify: `crates/mrd-ipc/src/lib.rs`
- Test: `crates/mrd-ipc/tests/contracts.rs`

**Step 1: Write failing contract tests**

Add tests near `serialize_deserialize_list_remote_capture_sources`:

```rust
#[test]
fn serialize_deserialize_list_local_capture_sources() {
    let request = IpcRequest::ListLocalCaptureSources {
        include_previews: false,
        limit: Some(24),
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("ListLocalCaptureSources"));
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_local_capture_source_list_response() {
    let response = IpcResponse::LocalCaptureSourceList {
        sources: vec![test_capture_source()],
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("LocalCaptureSourceList"));
    assert!(json.contains("windows:window:0x1234"));
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized, response);
}
```

Also add `IpcRequest::ListLocalCaptureSources { include_previews: true, limit: Some(16) }` to `serialize_deserialize_all_request_types`.

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-ipc serialize_deserialize_list_local_capture_sources -- --nocapture
```

Expected: compile failure because `ListLocalCaptureSources` and `LocalCaptureSourceList` do not exist.

**Step 3: Implement minimal IPC variants**

In `IpcRequest`, add after `ConfigureMediaAdaptation` and before remote capture source requests:

```rust
/// List selectable capture sources on the local service host.
ListLocalCaptureSources {
    include_previews: bool,
    limit: Option<u32>,
},
```

In `IpcResponse`, add before `CaptureSourceList`:

```rust
/// Local selectable capture sources returned by mrd-service.
LocalCaptureSourceList {
    sources: Vec<CaptureSource>,
},
```

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-ipc serialize_deserialize_list_local_capture_sources -- --nocapture
cargo test -p mrd-ipc serialize_deserialize_local_capture_source_list_response -- --nocapture
cargo test -p mrd-ipc serialize_deserialize_all_request_types -- --nocapture
```

Expected: all pass.

**Step 5: Commit**

```powershell
git add crates/mrd-ipc/src/lib.rs crates/mrd-ipc/tests/contracts.rs
git commit -m "feat: add local capture source IPC contract"
```

---

### Task 2: mrd-service Handler And Web Bridge Allowlist

**Files:**
- Modify: `apps/mrd-service/src/ipc_server.rs`
- Modify: `apps/mrd-service/src/web_bridge.rs`

**Step 1: Write failing service tests**

In `apps/mrd-service/src/ipc_server.rs` tests module, add a unit test that dispatches the new request and accepts either a list or a platform-specific error:

```rust
#[tokio::test]
async fn list_local_capture_sources_returns_local_response_or_error() {
    let server = test_server();
    let response = server
        .handle_request(IpcRequest::ListLocalCaptureSources {
            include_previews: false,
            limit: Some(4),
        })
        .await;

    match response {
        IpcResponse::LocalCaptureSourceList { sources } => {
            assert!(sources.len() <= 4);
        }
        IpcResponse::Error { code, message } => {
            assert_eq!(code, "CAPTURE_SOURCE_LIST_FAILED");
            assert!(!message.trim().is_empty());
        }
        other => panic!("unexpected response: {other:?}"),
    }
}
```

In `apps/mrd-service/src/web_bridge.rs` tests, add:

```rust
#[test]
fn web_bridge_allows_local_capture_source_listing() {
    assert!(is_ipc_request_allowed(&IpcRequest::ListLocalCaptureSources {
        include_previews: false,
        limit: Some(24),
    }));
}
```

**Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p mrd-service list_local_capture_sources_returns_local_response_or_error -- --nocapture
cargo test -p mrd-service web_bridge_allows_local_capture_source_listing -- --nocapture
```

Expected: compile failure or match failure because the request is not handled or allowed.

**Step 3: Implement handler**

In `IpcServer::handle_request`, add a match arm before remote capture source handling:

```rust
IpcRequest::ListLocalCaptureSources {
    include_previews,
    limit,
} => match crate::capture_source::list_capture_sources(include_previews, limit) {
    Ok(sources) => IpcResponse::LocalCaptureSourceList { sources },
    Err(error) => IpcResponse::Error {
        code: "CAPTURE_SOURCE_LIST_FAILED".to_string(),
        message: error.to_string(),
    },
},
```

In `web_bridge.rs`, add to `is_ipc_request_allowed`:

```rust
| IpcRequest::ListLocalCaptureSources { .. }
```

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service list_local_capture_sources_returns_local_response_or_error -- --nocapture
cargo test -p mrd-service web_bridge_allows_local_capture_source_listing -- --nocapture
```

Expected: both pass.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/ipc_server.rs apps/mrd-service/src/web_bridge.rs
git commit -m "feat: serve local capture source list"
```

---

### Task 3: Rdesk Tauri And TypeScript Local Source API

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/main.rs`
- Modify: `apps/Rdesk/src/app/adapters/tauri/commands.ts`
- Modify: `apps/Rdesk/src/app/services/ipcSessionService.ts`
- Test: `apps/Rdesk/src/app/adapters/tauri/contract.test.ts`
- Test: `apps/Rdesk/src/app/adapters/tauri/commands.webBridge.test.ts`

**Step 1: Write failing frontend adapter tests**

In `contract.test.ts`, near the remote capture source tests:

```ts
it('ipc_list_local_capture_sources calls correct command with args', async () => {
  const mockInvoke = getMockInvoke();
  mockInvoke.mockResolvedValue([
    {
      id: 'windows:display-shared:1',
      platform: 'windows',
      source_kind: 'display_shared',
      title: 'Display 2 (D3D11 shared copy)',
      class_name: 'DXGIShared:\\\\.\\DISPLAY2',
      width: 3840,
      height: 2160,
      process_id: 0,
      app_name: 'Display',
      bundle_identifier: null,
      preview_data_url: null,
      preview_width: null,
      preview_height: null,
    },
  ]);

  await adapter.ipcListLocalCaptureSources(false, 24);

  expect(mockInvoke).toHaveBeenCalledWith('ipc_list_local_capture_sources', {
    includePreviews: false,
    limit: 24,
  });
});
```

In `commands.webBridge.test.ts`, force web bridge and assert the JSON request contains `ListLocalCaptureSources`:

```ts
it('uses the browser service bridge for local capture source listing outside Tauri', async () => {
  const fetchMock = vi.fn().mockResolvedValueOnce({
    ok: true,
    json: async () => ({
      response: {
        type: 'LocalCaptureSourceList',
        sources: [{ id: 'windows:display-shared:1', platform: 'windows', source_kind: 'display_shared' }],
      },
    }),
  });
  vi.stubGlobal('fetch', fetchMock);

  const result = await ipcListLocalCaptureSources(false, 24);

  expect(result.ok).toBe(true);
  expect(fetchMock).toHaveBeenCalledWith(
    'http://127.0.0.1:9532/ipc',
    expect.objectContaining({
      method: 'POST',
      body: expect.stringContaining('"type":"ListLocalCaptureSources"'),
    })
  );
});
```

Adjust the minimal mocked source fields to match existing test helpers if the file uses stricter typing.

**Step 2: Run tests to verify failure**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/adapters/tauri/contract.test.ts src/app/adapters/tauri/commands.webBridge.test.ts
```

Expected: TypeScript compile/test failure because `ipcListLocalCaptureSources` does not exist.

**Step 3: Implement Tauri command**

In `apps/Rdesk/src-tauri/src/main.rs`, add:

```rust
/// List local capture sources through mrd-service IPC.
#[tauri::command]
async fn ipc_list_local_capture_sources(
    include_previews: bool,
    limit: Option<u32>,
) -> Result<Vec<mrd_ipc::CaptureSource>, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::ListLocalCaptureSources {
            include_previews,
            limit,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::LocalCaptureSourceList { sources } => Ok(sources),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}
```

Add `ipc_list_local_capture_sources` to `tauri::generate_handler![...]`.

**Step 4: Implement TypeScript adapter and service helper**

In `commands.ts`, add near `ipcListRemoteCaptureSources`:

```ts
export async function ipcListLocalCaptureSources(
  includePreviews = true,
  limit?: number
): Promise<AdapterResult<CaptureSource[]>> {
  const args = {
    includePreviews,
    ...(limit === undefined ? {} : { limit }),
  };
  return invokeBridgeOrTauri<CaptureSource[]>(
    'ipc_list_local_capture_sources',
    args,
    {
      type: 'ListLocalCaptureSources',
      include_previews: includePreviews,
      limit: limit ?? null,
    },
    responseField<CaptureSource[]>('sources')
  );
}
```

In `ipcSessionService.ts`, add:

```ts
export const listLocalCaptureSources = async (
  includePreviews = true,
  limit?: number
): Promise<CaptureSource[]> => {
  const result = await tauriAdapter.ipcListLocalCaptureSources(includePreviews, limit);
  return unwrapAdapterResult(result);
};
```

**Step 5: Run tests**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/adapters/tauri/contract.test.ts src/app/adapters/tauri/commands.webBridge.test.ts
```

Expected: all selected tests pass.

**Step 6: Commit**

```powershell
git add apps/Rdesk/src-tauri/src/main.rs apps/Rdesk/src/app/adapters/tauri/commands.ts apps/Rdesk/src/app/services/ipcSessionService.ts apps/Rdesk/src/app/adapters/tauri/contract.test.ts apps/Rdesk/src/app/adapters/tauri/commands.webBridge.test.ts
git commit -m "feat: expose local capture source listing to rdesk"
```

---

### Task 4: Browser Preview Source-Id Contract In mrd-service

**Files:**
- Modify: `apps/mrd-service/src/browser_webrtc_preview.rs`
- Modify: `apps/mrd-service/src/browser_webcodecs_preview.rs`

**Step 1: Write failing pure parser tests**

In each file tests module, or in a small shared test module if one already exists, add tests for the source-id validation helper. Prefer one helper per file unless a local shared module already exists.

Target behavior:

```rust
#[test]
fn browser_preview_source_id_accepts_empty_as_primary() {
    assert_eq!(
        parse_browser_preview_display_source_id(None).unwrap(),
        BrowserPreviewDisplaySource::Primary
    );
    assert_eq!(
        parse_browser_preview_display_source_id(Some("   ")).unwrap(),
        BrowserPreviewDisplaySource::Primary
    );
}

#[test]
fn browser_preview_source_id_accepts_windows_display_sources() {
    assert_eq!(
        parse_browser_preview_display_source_id(Some("windows:display-shared:1")).unwrap(),
        BrowserPreviewDisplaySource::DisplaySourceId("windows:display-shared:1".to_string())
    );
    assert_eq!(
        parse_browser_preview_display_source_id(Some("windows:display:0")).unwrap(),
        BrowserPreviewDisplaySource::DisplaySourceId("windows:display:0".to_string())
    );
}

#[test]
fn browser_preview_source_id_rejects_windows() {
    let error = parse_browser_preview_display_source_id(Some("windows:window:0x1234"))
        .unwrap_err();
    assert!(error.contains("display source"));
}
```

Also add serde tests:

```rust
#[test]
fn browser_webrtc_preview_start_deserializes_source_id() {
    let request: BrowserWebrtcPreviewStartRequest = serde_json::from_str(
        r#"{"session_id":"s1","offer_sdp":"v=0","source_id":"windows:display-shared:1"}"#,
    )
    .unwrap();

    assert_eq!(request.source_id.as_deref(), Some("windows:display-shared:1"));
}
```

For WebCodecs:

```rust
#[test]
fn browser_webcodecs_preview_start_deserializes_source_id() {
    let message: BrowserWebcodecsPreviewControlMessage = serde_json::from_str(
        r#"{"type":"start","session_id":"s1","source_id":"windows:display-shared:1"}"#,
    )
    .unwrap();

    let BrowserWebcodecsPreviewControlMessage::Start(request) = message else {
        panic!("expected start message");
    };
    assert_eq!(request.source_id.as_deref(), Some("windows:display-shared:1"));
}
```

**Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p mrd-service browser_preview_source_id -- --nocapture
cargo test -p mrd-service browser_webrtc_preview_start_deserializes_source_id -- --nocapture
cargo test -p mrd-service browser_webcodecs_preview_start_deserializes_source_id -- --nocapture
```

Expected: compile failure because request fields and helper do not exist.

**Step 3: Add request fields**

Add to `BrowserWebrtcPreviewStartRequest` and `BrowserWebcodecsPreviewStartRequest`:

```rust
#[serde(default)]
pub source_id: Option<String>,
```

**Step 4: Add source parser and DXGI opener**

Add this helper shape to both modules, or factor it into a small `apps/mrd-service/src/browser_preview_capture.rs` module if duplication grows:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum BrowserPreviewDisplaySource {
    Primary,
    DisplaySourceId(String),
}

fn parse_browser_preview_display_source_id(
    source_id: Option<&str>,
) -> Result<BrowserPreviewDisplaySource, String> {
    let Some(source_id) = source_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(BrowserPreviewDisplaySource::Primary);
    };

    let parts = source_id.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["windows", "display", index] | ["windows", "display-shared", index]
            if index.parse::<u32>().is_ok() =>
        {
            Ok(BrowserPreviewDisplaySource::DisplaySourceId(source_id.to_string()))
        }
        _ => Err(format!(
            "browser preview source_id must be a Windows display source, got {source_id}"
        )),
    }
}

#[cfg(windows)]
fn open_browser_preview_dxgi_capture(
    source_id: Option<&str>,
) -> Result<DxgiSharedTextureCapture, String> {
    match parse_browser_preview_display_source_id(source_id)? {
        BrowserPreviewDisplaySource::Primary => DxgiSharedTextureCapture::new_primary()
            .map_err(|error| format!("DXGI capture unavailable: {error}")),
        BrowserPreviewDisplaySource::DisplaySourceId(source_id) => {
            let device_name = crate::display_mode::display_device_name_for_source_id(&source_id)
                .map_err(|error| format!("resolve display source {source_id} failed: {error}"))?;
            DxgiSharedTextureCapture::new_for_device_name(&device_name).map_err(|error| {
                format!("DXGI capture unavailable for {source_id} ({device_name}): {error}")
            })
        }
    }
}
```

**Step 5: Wire WebRTC validation and sender**

Change:

```rust
validate_browser_preview_sender(fps, bitrate_bps, request.width, request.height)?;
```

to:

```rust
validate_browser_preview_sender(
    request.source_id.as_deref(),
    fps,
    bitrate_bps,
    request.width,
    request.height,
)?;
```

Pass `request.source_id.clone()` into `spawn_local_capture_sender(...)`, then into `run_local_capture_sender(...)`.

In `validate_browser_preview_sender` and `run_local_capture_sender`, replace `DxgiSharedTextureCapture::new_primary()` with `open_browser_preview_dxgi_capture(source_id.as_deref())`.

Update the start log to include `source_id.unwrap_or("<primary>")`.

**Step 6: Wire WebCodecs sender**

In `run_browser_webcodecs_capture_sender`, replace `DxgiSharedTextureCapture::new_primary()` with:

```rust
let mut capture = match open_browser_preview_dxgi_capture(request.source_id.as_deref()) {
    Ok(capture) => capture,
    Err(error) => {
        send_error(
            &outbound,
            &session_id,
            format!("WebCodecs DXGI capture failed: {error}"),
        );
        running.store(false, Ordering::Relaxed);
        return;
    }
};
```

Log `request.source_id.as_deref().unwrap_or("<primary>")`.

**Step 7: Run tests**

Run:

```powershell
cargo test -p mrd-service browser_preview_source_id -- --nocapture
cargo test -p mrd-service browser_webrtc_preview_start_deserializes_source_id -- --nocapture
cargo test -p mrd-service browser_webcodecs_preview_start_deserializes_source_id -- --nocapture
```

Expected: all pass.

**Step 8: Commit**

```powershell
git add apps/mrd-service/src/browser_webrtc_preview.rs apps/mrd-service/src/browser_webcodecs_preview.rs
git commit -m "feat: target browser previews to selected display"
```

---

### Task 5: Rdesk Local Capture Source Selector

**Files:**
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Test: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx`

**Step 1: Write failing component tests**

Add a local display source fixture:

```ts
const localDisplaySources = [
  {
    ...remoteDisplaySource,
    id: "windows:display-shared:0",
    title: "Display 1 (D3D11 shared copy)",
    width: 2560,
    height: 1440,
  },
  {
    ...remoteDisplaySource,
    id: "windows:display-shared:1",
    title: "Display 2 (D3D11 shared copy)",
    width: 3840,
    height: 2160,
  },
];
```

Add a test that renders a local preview session, opens settings, refreshes local sources, and verifies the selector is visible:

```ts
it("shows local display source selection for local preview sessions", async () => {
  const mockInvoke = getMockInvoke();
  mockInvoke.mockImplementation((command: string) => {
    if (command === "test_get_capabilities") return Promise.resolve(windowsCapabilities());
    if (command === "ipc_list_local_capture_sources") return Promise.resolve(localDisplaySources);
    if (command === "current_remote_display_window_context") return Promise.resolve(null);
    if (command === "get_system_resource_snapshot") {
      return Promise.resolve({ target_found: false, target_name: "mrd-service" });
    }
    return Promise.resolve(null);
  });

  renderRemoteDisplay("local-preview");
  fireEvent.click(await screen.findByRole("button", { name: /测试配置|settings/i }));
  fireEvent.click(await screen.findByRole("button", { name: /刷新捕获源/ }));

  expect(await screen.findByText(/Display 2/)).toBeInTheDocument();
  expect(screen.getByText(/3840x2160/)).toBeInTheDocument();
});
```

Use existing test labels if the settings button has a different accessible name.

**Step 2: Run test to verify failure**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx -t "local display source selection"
```

Expected: fails because local sessions return early and no local source listing command is called.

**Step 3: Implement local source state**

In `RemoteDisplayWindowPage.tsx`:

- Import `listLocalCaptureSources` from `ipcSessionService`.
- Keep the existing `captureSources` and `captureSourceSelection` state, but allow it to represent local source selection when `isLocalPipelinePreview` is true.
- Add a helper:

```ts
function localCaptureSourceSelection(source: CaptureSource): CaptureSourceSelection {
  return {
    session_id: "local-preview",
    source,
    status: "selected",
    reason: null,
  };
}
```

- Change `handleRefreshRemoteCaptureSources` into a neutral `handleRefreshCaptureSources`:

```ts
const sources = isLocalPipelinePreview
  ? await listLocalCaptureSources(false, 24)
  : await listRemoteCaptureSources(sessionId, false, 24);
```

- Change hydration to use `listLocalCaptureSources(true, ...)` for local sessions.
- Change select handler:

```ts
if (isLocalPipelinePreview) {
  setCaptureSourceSelection(localCaptureSourceSelection(source));
  setTestMessage(
    `本机捕获源已切换: ${captureSourceKindLabel(source.source_kind)} / ${source.title}`
  );
  return;
}
```

- Update UI section condition from `!isLocalPipelinePreview` to always render the source selector, with title text switching between `本机捕获源` and `远端捕获源`.
- Keep disabled state tied to `localRenderSwitchLocked` or `captureSourcesLoading`.
- Make `pickPreferredCaptureSource` prefer `display_shared` first, then `display`, then `window`.

**Step 4: Run component test**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx -t "local display source selection"
```

Expected: pass.

**Step 5: Commit**

```powershell
git add apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx
git commit -m "feat: show local display source selector"
```

---

### Task 6: Propagate Selected Source To WebRTC And WebCodecs Diagnostic Preview

**Files:**
- Modify: `apps/Rdesk/src/app/adapters/tauri/commands.ts`
- Modify: `apps/Rdesk/src/app/workers/webCodecsPreview.worker.ts`
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Test: `apps/Rdesk/src/app/adapters/tauri/contract.test.ts`
- Test: `apps/Rdesk/src/app/adapters/tauri/commands.webBridge.test.ts`
- Test: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx`

**Step 1: Write failing adapter tests**

In `contract.test.ts`, add to WebRTC preview command coverage:

```ts
await adapter.browserWebrtcPreviewStart({
  sessionId: "local-display-test-1",
  offerSdp: "offer-sdp",
  sourceId: "windows:display-shared:1",
});

expect(mockInvoke).toHaveBeenCalledWith(
  "browser_webrtc_preview_start",
  expect.objectContaining({
    sourceId: "windows:display-shared:1",
  })
);
```

In `commands.webBridge.test.ts`, extend the service bridge WebRTC preview start test:

```ts
const start = await browserWebrtcPreviewStart({
  sessionId: 'local-display-test-1',
  offerSdp: 'offer-sdp',
  fps: 120,
  h264Profile: 'high',
  sourceId: 'windows:display-shared:1',
});

expect(fetchMock).toHaveBeenNthCalledWith(
  1,
  'http://127.0.0.1:9532/browser/webrtc-preview/start',
  expect.objectContaining({
    body: expect.stringContaining('"source_id":"windows:display-shared:1"'),
  })
);
```

**Step 2: Write failing UI propagation tests**

In `RemoteDisplayWindowPage.test.tsx`, mock local source list, select Display 2, start WebRTC preview, and assert `browser_webrtc_preview_start` receives `sourceId: "windows:display-shared:1"`.

For WebCodecs, stub `Worker` with a fake class that records `postMessage` and assert the start message contains:

```ts
expect(workerPostMessage).toHaveBeenCalledWith(
  expect.objectContaining({
    type: "start",
    sourceId: "windows:display-shared:1",
  }),
  expect.anything()
);
```

If existing tests do not expose a clean start button, test only the exported adapter/worker contract in this task and leave component coverage to the selector test.

**Step 3: Run tests to verify failure**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/adapters/tauri/contract.test.ts src/app/adapters/tauri/commands.webBridge.test.ts src/app/components/RemoteDisplayWindowPage.test.tsx -t "source"
```

Expected: compile/test failure because `sourceId` is not part of the preview command or worker start message.

**Step 4: Implement WebRTC adapter propagation**

In `commands.ts`, add to `browserWebrtcPreviewStart` params:

```ts
sourceId?: string;
```

In service bridge body:

```ts
source_id: params.sourceId ?? null,
```

In Tauri invoke args:

```ts
sourceId: params.sourceId ?? null,
```

In `apps/Rdesk/src-tauri/src/main.rs`, add `source_id: Option<String>` to the `browser_webrtc_preview_start` command signature. The in-process Tauri command may ignore it for now if the local harness source wiring is done in Task 7, but keep the parameter so the contract is stable:

```rust
let _ = (width, height, bitrate_mbps, source_id);
```

**Step 5: Implement WebCodecs worker propagation**

In `webCodecsPreview.worker.ts`, add to `StartMessage`:

```ts
sourceId?: string;
```

In socket start JSON:

```ts
source_id: message.sourceId ?? null,
```

In `RemoteDisplayWindowPage.tsx`, compute:

```ts
const selectedLocalSourceId = isLocalPipelinePreview
  ? captureSourceSelection?.source.id
  : undefined;
```

Pass to WebRTC:

```ts
sourceId: selectedLocalSourceId,
```

Pass to worker start message:

```ts
sourceId: selectedLocalSourceId,
```

For main-thread WebCodecs fallback, include `source_id` in the WebSocket start JSON.

**Step 6: Run tests**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/adapters/tauri/contract.test.ts src/app/adapters/tauri/commands.webBridge.test.ts src/app/components/RemoteDisplayWindowPage.test.tsx -t "source"
```

Expected: pass.

**Step 7: Commit**

```powershell
git add apps/Rdesk/src-tauri/src/main.rs apps/Rdesk/src/app/adapters/tauri/commands.ts apps/Rdesk/src/app/workers/webCodecsPreview.worker.ts apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx apps/Rdesk/src/app/adapters/tauri/contract.test.ts apps/Rdesk/src/app/adapters/tauri/commands.webBridge.test.ts apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx
git commit -m "feat: pass selected display to browser previews"
```

---

### Task 7: Local Harness Source-Id Plumbing For Native Diagnostic Runs

This task keeps the older single-process local harness from contradicting the new selector. It is not the B performance baseline, but it prevents local native diagnostic runs from silently staying on primary display.

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`
- Modify: `apps/Rdesk/src-tauri/src/test_orchestrator.rs`
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Test: `apps/Rdesk/src-tauri/src/test_orchestrator.rs`
- Test: `apps/Rdesk/src-tauri/src/test_harness.rs`

**Step 1: Write failing Rust tests**

In `test_orchestrator.rs`, add a test near config mapping tests:

```rust
#[test]
fn harness_config_preserves_source_id() {
    let config = TestConfigData {
        source_id: Some("windows:display-shared:1".to_string()),
        ..Default::default()
    };

    let harness = harness_config_from_data(&config);

    assert_eq!(harness.source_id.as_deref(), Some("windows:display-shared:1"));
}
```

In `test_harness.rs`, add pure tests for source id parsing:

```rust
#[test]
fn parse_display_index_accepts_windows_display_source_ids() {
    assert_eq!(
        parse_display_index(Some("windows:display-shared:1")).unwrap(),
        1
    );
    assert_eq!(parse_display_index(Some("windows:display:0")).unwrap(), 0);
}
```

If adding a DXGI device-name resolver helper with an injectable target slice, test:

```rust
#[test]
fn dxgi_source_id_selects_matching_target_index() {
    let targets = vec![
        test_dxgi_target("\\\\.\\DISPLAY1"),
        test_dxgi_target("\\\\.\\DISPLAY2"),
    ];

    assert_eq!(
        dxgi_device_name_for_source_id(Some("windows:display-shared:1"), &targets)
            .unwrap()
            .as_deref(),
        Some("\\\\.\\DISPLAY2")
    );
}
```

**Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p app harness_config_preserves_source_id -- --nocapture
cargo test -p app parse_display_index_accepts_windows_display_source_ids -- --nocapture
```

Expected: compile failure because `TestConfig` lacks `source_id`, and/or parser behavior is not covered.

**Step 3: Implement Rust config plumbing**

In `test_harness.rs` `TestConfig`, add:

```rust
pub source_id: Option<String>,
```

Set `source_id: None` in `Default`.

In `test_orchestrator.rs` `harness_config_from_data`, add:

```rust
source_id: config.source_id.clone(),
display_id: config.display_id.clone().or_else(|| config.source_id.clone()),
```

Prefer keeping `display_id` as explicit override if both are present:

```rust
display_id: config.display_id.clone().or_else(|| config.source_id.clone()),
```

In `test_harness.rs`, for WinRT monitor capture paths, use:

```rust
let display_ref = config.display_id.as_deref().or(config.source_id.as_deref());
let monitor_index = parse_display_index(display_ref)?;
```

For DXGI shared texture capture, use `source_id` only when present:

```rust
let mut capture = if let Some(source_id) = config.source_id.as_deref() {
    let device_name = dxgi_device_name_for_source_id(source_id)?;
    DxgiSharedTextureCapture::new_for_device_name(&device_name)
} else {
    DxgiSharedTextureCapture::new_primary()
}
.map_err(|e| anyhow::anyhow!("DXGI shared texture capture init failed: {:?}", e))?;
```

Implement `dxgi_device_name_for_source_id(source_id: &str) -> Result<String>` on Windows by parsing the display index and selecting the same index from `mrd_capture_dxgi::enumerate_dxgi_output_targets()`. If no target exists, return an explicit error naming the source id.

**Step 4: Implement frontend config propagation**

In `RemoteDisplayWindowPage.tsx` `buildTestConfig`, include selected local source fields:

```ts
const selectedSource = isLocalPipelinePreview ? captureSourceSelection?.source : null;
...
...(selectedSource
  ? {
      source_id: selectedSource.id,
      source_kind: selectedSource.source_kind,
      display_id: selectedSource.id,
    }
  : {}),
```

Add `captureSourceSelection` and `isLocalPipelinePreview` to the dependency list.

**Step 5: Run tests**

Run:

```powershell
cargo test -p app harness_config_preserves_source_id -- --nocapture
cargo test -p app parse_display_index_accepts_windows_display_source_ids -- --nocapture
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx -t "local display source"
```

Expected: pass.

**Step 6: Commit**

```powershell
git add apps/Rdesk/src-tauri/src/test_harness.rs apps/Rdesk/src-tauri/src/test_orchestrator.rs apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx
git commit -m "feat: carry selected display into local harness"
```

---

### Task 8: B Baseline Verification Runbook

**Files:**
- Modify if needed: `tests/benchmarks/scripts/paired_lan_canary_common.ps1`
- Modify if needed: `tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1`
- Optional doc update: `docs/plans/2026-05-23-dx11-h264-display-selection-optimization-design.md`

Only edit scripts if tests reveal a missing profile or source-id report regression. They already support `-CaptureSourceId` and have `2k144` plus `4k120`.

**Step 1: Run script unit tests**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Expected: pass and confirm profiles include `2k144`, `2k144_adaptive`, and `4k120`.

**Step 2: Build baseline binaries**

Run:

```powershell
cargo build -p mrd-service -p app
```

Expected: build succeeds.

**Step 3: Enumerate local capture source ids**

Use Rdesk local selector from Task 5, or call the service bridge IPC endpoint if already running. Record which source id maps to the 2K144 display and which maps to the 4K120 display.

Expected examples:

- `windows:display-shared:0` - 2560x1440, 144 Hz
- `windows:display-shared:1` - 3840x2160, 120 Hz

The actual ordering can be swapped. Do not assume primary display is the 2K monitor.

**Step 4: Run B baseline canary for the 2K144 display**

Replace `windows:display-shared:0` with the actual 2K144 source id:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 2k144 -Codec h264 -CaptureSourceId windows:display-shared:0 -DurationSecs 10 -NoBuild
```

Expected: report contains:

- `requested_capture_source_id` equal to the requested source id.
- `actual_capture_source_id` or selected source metadata matching the same display.
- H.264 selected codec.
- No primary-display fallback message.

**Step 5: Run B baseline canary for the 4K120 display**

Replace `windows:display-shared:1` with the actual 4K120 source id:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 4k120 -Codec h264 -CaptureSourceId windows:display-shared:1 -DurationSecs 10 -NoBuild
```

Expected: report contains the requested 4K display id and selected profile is 3840x2160 @ 120.

**Step 6: Commit only if scripts/docs changed**

```powershell
git add tests/benchmarks/scripts/paired_lan_canary_common.ps1 tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 docs/plans/2026-05-23-dx11-h264-display-selection-optimization-design.md
git commit -m "test: document dx11 h264 display canaries"
```

---

### Task 9: Full Verification

**Files:** no edits unless a command reveals a bug.

**Step 1: Format**

Run:

```powershell
cargo fmt --all -- --check
```

Expected: pass. If it fails, run `cargo fmt --all`, inspect `git diff`, and commit the formatting with the relevant task or a final `style:` commit.

**Step 2: Rust protocol and service tests**

Run:

```powershell
cargo test -p mrd-ipc
cargo test -p mrd-service browser_webrtc_preview -- --nocapture
cargo test -p mrd-service browser_webcodecs_preview -- --nocapture
cargo test -p mrd-service list_local_capture_sources -- --nocapture
```

Expected: pass.

**Step 3: Tauri harness tests**

Run:

```powershell
cargo test -p app harness_config_preserves_source_id -- --nocapture
cargo test -p app parse_display_index_accepts_windows_display_source_ids -- --nocapture
```

Expected: pass.

**Step 4: Frontend tests and type check**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx src/app/adapters/tauri/commands.webBridge.test.ts src/app/adapters/tauri/contract.test.ts
pnpm --dir apps/Rdesk type-check
```

Expected: pass.

**Step 5: Benchmark script tests**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Expected: pass.

**Step 6: Hardware canaries**

Run the two commands from Task 8 on the actual 2K144 and 4K120 displays. If they cannot run in the current environment, record that they were not run and why. Do not claim B baseline completion without canary output.

**Step 7: Final commit**

If all verification passes and there are unstaged changes:

```powershell
git status --short
git add <changed-files>
git commit -m "feat: optimize dx11 h264 display selection"
```

Expected: clean working tree or only intentionally untracked reports.

---

## Acceptance Criteria

- Rdesk local preview settings show selectable local displays when multiple monitors are attached.
- The selected local display id is sent to WebRTC and WebCodecs diagnostic preview start requests as `source_id`.
- `mrd-service` browser preview opens DXGI capture for the requested display instead of always using primary.
- Local harness diagnostic runs carry `source_id` and avoid primary fallback when possible.
- B baseline runs support both the 2K144 and 4K120 displays through `-CaptureSourceId`.
- The final report states exact commands run, exact source ids used, and whether hardware canaries passed.
