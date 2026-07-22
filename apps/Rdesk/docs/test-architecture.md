# Frontend Test Architecture Design

## Problem Statement

The current frontend test suite has **5 test files, 44 tests, all passing**, yet it failed to catch critical runtime errors:
- Undefined function calls in `RemoteSessionPage.tsx` (ReferenceError)
- Deprecated service usage in `SettingsModal.tsx`
- Contract drift between frontend services and backend handlers

**Root cause**: Test layering mismatch. Current tests are mostly service-layer mocks with static HTML assertions, providing no coverage for:
- Component lifecycle (useEffect, cleanup)
- User interactions (clicks, form submissions)
- Tauri command failures
- Route-level integration

---

## Current State Analysis

### Test Files (5)

| File | Tests | Type | Coverage Gap |
|------|-------|------|--------------|
| `realtimeService.test.ts` | 11 | Service mock | Tests invoke() calls, but service now throws |
| `renderWindowService.test.ts` | 10 | Service mock | Tests invoke() calls, but commands removed |
| `renderHostService.test.ts` | 4 | Service mock | Tests invoke() calls, but commands removed |
| `realtimeService.test.ts` | 18 | Service mock | Tests deprecated functions |
| `RealtimeSessionCard.test.tsx` | 1 | Static HTML | Only renderToStaticMarkup string assertion |

### Missing Test Types

- ❌ Component lifecycle tests (useEffect, mounting/unmounting)
- ❌ User interaction tests (clicks, inputs, navigation)
- ❌ Error path tests (Tauri failures, deprecated commands)
- ❌ Integration tests (route → component → service → backend)

---

## Proposed Test Architecture

### Three-Layer Test Pyramid

```
         ┌─────────────────┐
         │   E2E Tests     │  ← Tauri integration (few, slow)
         │   (Tauri)       │
         ├─────────────────┤
         │  Component      │  ← DOM + interactions (most)
         │    Tests        │
         ├─────────────────┤
         │   Service       │  ← Pure logic mocks (fastest)
         │    Tests        │  ← Current focus
         └─────────────────┘
```

### 1. Service Tests (Unit)

**Purpose**: Test pure business logic, IPC contracts, type conversions

**Example**:
```typescript
describe("ipcSessionService", () => {
  it("throws when startSession fails", async () => {
    mockInvoke.mockRejectedValue(new Error("IPC timeout"));
    await expect(startSession("s1", "d1")).rejects.toThrow("IPC timeout");
  });
});
```

**Tools**: Vitest, vi.mock()

### 2. Component Tests (Integration)

**Purpose**: Test component behavior, user interactions, error handling

**Required setup**:
- `@testing-library/react` - Component rendering
- `happy-dom` or `jsdom` - DOM environment
- `@testing-library/user-event` - User interactions

**Example**:
```typescript
describe("SettingsModal", () => {
  it("shows deprecation notice when NVDEC probe fails", async () => {
    mockInvoke.mockRejectedValue(new Error("moved to mrd-service"));
    render(<SettingsModal open={true} onClose={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/功能已迁移到 mrd-service/)).toBeInTheDocument();
    });
  });

  it("does not call deprecated services on mount", async () => {
    const spy = vi.spyOn(realtimeService, "getNvdecRuntimeProbe");
    render(<SettingsModal open={true} onClose={vi.fn()} />);

    await waitFor(() => {
      expect(spy).not.toHaveBeenCalled();
    });
  });
});
```

**Tools**: Vitest, @testing-library, happy-dom

### 3. Contract Tests (Tauri Integration)

**Purpose**: Verify frontend ↔ Tauri command contracts

**Approach**: Create contract tests that verify:
- Command exists in invoke_handler
- Request/Response types match
- Error paths return correct format

**Example**:
```typescript
describe("Tauri Command Contracts", () => {
  it("ipc_start_session accepts correct payload", () => {
    // Verify the command signature matches frontend usage
    expect(ipcStartSession).toAcceptCommand({
      sessionId: "string",
      targetDeviceId: "string",
      transportKind: "webrtc" | "quic"
    });
  });
});
```

---

## Implementation Plan

### Phase 1: Fix Type Checking (Done ✅)

- [x] Create `tsconfig.json`
- [x] Create `tsconfig.node.json`
- [x] Add `type-check` script to package.json
- [x] Add TypeScript to devDependencies

### Phase 2: Test Infrastructure Setup

**Files to create**:

1. **`src/test/setup.ts`** - Test globals and mocks
```typescript
import { expect, afterEach, vi } from 'vitest';
import * as matchers from '@testing-library/jest-dom/matchers';

expect.extend(matchers);

// Cleanup after each test
afterEach(() => {
  vi.clearAllMocks();
});
```

2. **`src/test/mocks/tauri.ts`** - Tauri API mocks
```typescript
import { vi } from 'vitest';

export const mockInvoke = vi.fn();

vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: mockInvoke,
  invoke: () => mockInvoke,
}));
```

3. **`src/test/utils/test-renderers.tsx`** - Custom render utils
```typescript
import { render } from '@testing-library/react';
import { BrowserRouter } from 'react-router';

export function renderWithRouter(component: React.ReactElement) {
  return render(<BrowserRouter>{component}</BrowserRouter>);
}
```

### Phase 3: Component Test Examples

**Priority components to test**:

1. **`IpcSessionCard.test.tsx`** - New IPC-based session control
   - Device registration flow
   - Session start/stop buttons
   - Error handling

2. **`SettingsModal.test.tsx`** - Settings with deprecated features
   - Does NOT call deprecated services
   - Shows deprecation notices
   - Service status display

3. **`RemoteSessionPage.test.tsx`** - Remote session page
   - Does NOT call undefined functions
   - Handles disabled rendering features
   - Disconnect button works

### Phase 4: CI Integration

**Add to CI pipeline**:
```yaml
# Example: .github/workflows/frontend.yml
- name: Type check
  run: pnpm type-check

- name: Lint
  run: pnpm lint

- name: Unit tests
  run: pnpm test

- name: Component tests
  run: pnpm test --component
```

---

## Test File Naming Convention

| Pattern | Purpose | Example |
|---------|---------|---------|
| `*.service.test.ts` | Service layer unit tests | `ipcSessionService.test.ts` |
| `*.component.test.tsx` | Component behavior tests | `SettingsModal.component.test.tsx` |
| `*.contract.test.ts` | Contract/API tests | `tauri-contracts.contract.test.ts` |
| `*.integration.test.tsx` | Multi-component integration | `RemoteSessionFlow.integration.test.tsx` |

---

## Success Criteria

1. **Type checking catches undefined symbols**: `pnpm type-check` fails on ReferenceError risks
2. **Component tests catch deprecated usage**: Tests fail if deprecated services are called
3. **Coverage report meaningful**: Target 70%+ component coverage (statements)
4. **CI prevents regressions**: All checks must pass before merge

---

## Next Steps

1. Run `pnpm install` to add new dependencies
2. Run `pnpm type-check` to verify it catches errors
3. Create `src/test/setup.ts` with test configuration
4. Write first component test: `SettingsModal.component.test.tsx`
5. Update CI/CD to include type-check and component tests
