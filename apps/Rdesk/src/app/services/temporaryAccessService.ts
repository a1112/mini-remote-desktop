import * as adapter from '../adapters/tauri';
import type { AdapterResult } from '../adapters/tauri';
export type { TemporaryAccessStatus, TemporaryAccessSecret } from '../adapters/tauri';

function unwrap<T>(result: AdapterResult<T>): T {
  if (!result.ok) throw new Error(result.error.message);
  return result.value;
}
export const getTemporaryAccessStatus = async () => unwrap(await adapter.getTemporaryAccessStatus());
export const readTemporaryAccessPassword = async () => unwrap(await adapter.readTemporaryAccessPassword());
export const rotateTemporaryAccessPassword = async () => unwrap(await adapter.rotateTemporaryAccessPassword());
export const disableTemporaryAccess = async () => unwrap(await adapter.disableTemporaryAccess());
