/**
 * Test setup and global configuration
 *
 * This file is configured in vite.config.ts as the Vitest setupFiles entry.
 */

import { expect, afterEach, vi } from "vitest";
import * as matchers from "@testing-library/jest-dom/matchers";
import { resetTauriMock } from "./mocks/tauri";

const animationFrameTimers = new Map<number, ReturnType<typeof setTimeout>>();
let nextAnimationFrameId = 1;

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();

  get length() {
    return this.values.size;
  }

  clear() {
    this.values.clear();
  }

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  key(index: number) {
    return Array.from(this.values.keys())[index] ?? null;
  }

  removeItem(key: string) {
    this.values.delete(key);
  }

  setItem(key: string, value: string) {
    this.values.set(key, String(value));
  }
}

const testLocalStorage = new MemoryStorage();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: testLocalStorage,
});
Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: testLocalStorage,
});

// Extend Vitest's expect with jest-dom matchers
expect.extend(matchers);

// Cleanup after each test
afterEach(() => {
  for (const timeout of animationFrameTimers.values()) {
    clearTimeout(timeout);
  }
  animationFrameTimers.clear();
  vi.clearAllMocks();
  resetTauriMock();
});

// Mock window.matchMedia for components that use it
const mockMatchMedia = vi.fn((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: vi.fn(),
  removeListener: vi.fn(),
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
  dispatchEvent: vi.fn(),
}));
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: mockMatchMedia,
});

// Mock ResizeObserver
class MockResizeObserver {
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();
}

globalThis.ResizeObserver = MockResizeObserver as unknown as typeof ResizeObserver;

// Mock requestAnimationFrame
globalThis.requestAnimationFrame = vi.fn((cb: FrameRequestCallback): number => {
  const frameId = nextAnimationFrameId++;
  const timeout = setTimeout(() => {
    animationFrameTimers.delete(frameId);
    cb(performance.now());
  }, 0);
  animationFrameTimers.set(frameId, timeout);
  return frameId;
});
globalThis.cancelAnimationFrame = vi.fn((frameId: number): void => {
  const timeout = animationFrameTimers.get(frameId);
  if (timeout) {
    clearTimeout(timeout);
    animationFrameTimers.delete(frameId);
  }
});
