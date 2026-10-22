import type { ControlInputButton } from '../adapters/tauri/types';

export function browserPointerButton(button: number): ControlInputButton | null {
  return (['left', 'middle', 'right', 'x1', 'x2'] as const)[button] ?? null;
}

export function browserRemotePoint(x: number, y: number, rect: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>, frame: { width: number; height: number }): { x: number; y: number } | null {
  if (![x, y, rect.left, rect.top, rect.width, rect.height, frame.width, frame.height].every(Number.isFinite) || rect.width <= 0 || rect.height <= 0 || frame.width <= 0 || frame.height <= 0) return null;
  const scale = Math.min(rect.width / frame.width, rect.height / frame.height);
  const width = frame.width * scale;
  const height = frame.height * scale;
  const localX = x - rect.left - (rect.width - width) / 2;
  const localY = y - rect.top - (rect.height - height) / 2;
  if (localX < 0 || localY < 0 || localX > width || localY > height) return null;
  return { x: Math.min(frame.width - 1, Math.round(localX / scale)), y: Math.min(frame.height - 1, Math.round(localY / scale)) };
}

const keyCodes: Record<string, number> = {
  Backspace: 0x08, Tab: 0x09, Enter: 0x0d, Shift: 0x10, Control: 0x11, Alt: 0x12,
  Pause: 0x13, CapsLock: 0x14, Escape: 0x1b, ' ': 0x20, Spacebar: 0x20,
  PageUp: 0x21, PageDown: 0x22, End: 0x23, Home: 0x24, ArrowLeft: 0x25,
  ArrowUp: 0x26, ArrowRight: 0x27, ArrowDown: 0x28, PrintScreen: 0x2c,
  Insert: 0x2d, Delete: 0x2e, Meta: 0x5b, ContextMenu: 0x5d, NumLock: 0x90, ScrollLock: 0x91,
};
const physicalCodes: Record<string, number> = {
  NumpadMultiply: 0x6a, NumpadAdd: 0x6b, NumpadEqual: 0xbb, NumpadSubtract: 0x6d,
  NumpadDecimal: 0x6e, NumpadDivide: 0x6f, Semicolon: 0xba, Equal: 0xbb,
  Comma: 0xbc, Minus: 0xbd, Period: 0xbe, Slash: 0xbf, Backquote: 0xc0,
  BracketLeft: 0xdb, Backslash: 0xdc, BracketRight: 0xdd, Quote: 0xde,
};
export function browserVirtualKey(event: Pick<KeyboardEvent, 'key' | 'code'>): number | null {
  if (/^Key[A-Z]$/.test(event.code)) return event.code.charCodeAt(3);
  if (/^Digit[0-9]$/.test(event.code)) return event.code.charCodeAt(5);
  if (/^Numpad[0-9]$/.test(event.code)) return 0x60 + Number(event.code.slice(6));
  if (/^F(?:[1-9]|1[0-9]|2[0-4])$/.test(event.code)) return 0x6f + Number(event.code.slice(1));
  return physicalCodes[event.code] ?? keyCodes[event.key] ?? null;
}

export function browserWheelDelta(delta: number, mode: number, height: number): number {
  if (!Number.isFinite(delta)) return 0;
  const multiplier = mode === 1 ? 16 : mode === 2 ? Math.max(1, height) : 1;
  return Math.max(-32767, Math.min(32767, Math.trunc(-delta * multiplier)));
}
