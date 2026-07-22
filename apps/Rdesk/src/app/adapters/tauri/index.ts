/**
 * Tauri Adapter
 *
 * Centralized access point for all Tauri IPC commands.
 * This is the ONLY place in the frontend that should call invoke().
 *
 * Rules:
 * 1. All Tauri commands must be wrapped in commands.ts
 * 2. Command names MUST match the invoke_handler in main.rs
 * 3. When a command is removed/renamed in main.rs, update commands.ts
 * 4. Services should import from here, never from @tauri-apps/api/tauri directly
 * 5. Tests should mock the adapter functions, not invoke() directly
 */

export * from './types';
export * from './commands';
