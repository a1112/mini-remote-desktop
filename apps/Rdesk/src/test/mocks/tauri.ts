/**
 * Canonical Tauri mock for all tests
 *
 * This is the single source of truth for mocking Tauri commands in tests.
 * Page tests should NOT mock @tauri-apps/api/tauri directly.
 *
 * Usage:
 *   import { mockInvoke } from '@/test/mocks/tauri';
 *   mockInvoke.mockResolvedValue(result);
 */

import { vi } from 'vitest';

type TauriCommand = string;
type TauriArgs = Record<string, unknown>;

// Single mock instance that all tests use
const mockInvokeFn = vi.fn();

export const mockInvoke = mockInvokeFn;

// Reset mock between tests (called in setup files or beforeEach)
export const resetTauriMock = () => {
  mockInvokeFn.mockReset();
};

// Helper to set default return value
export const setDefaultTauriMock = (returnValue: unknown = undefined) => {
  mockInvokeFn.mockReturnValue(returnValue);
};

// Helper to mock a specific command
export const mockTauriCommand = (
  command: TauriCommand,
  response: unknown
) => {
  mockInvokeFn.mockImplementation((cmd: TauriCommand, args?: TauriArgs) => {
    if (cmd === command) {
      return response;
    }
    // Default to successful resolve for service commands
    return Promise.resolve(true);
  });
};

// Setup mock for @tauri-apps/api/tauri
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: mockInvokeFn,
}));

export const getMockInvoke = () => mockInvokeFn;
