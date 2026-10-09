import { describe, expect, it } from 'vitest';
import { browserPointerButton, browserRemotePoint, browserVirtualKey, browserWheelDelta } from './browserRemoteInput';

describe('browser input to native control protocol', () => {
  it('maps the visible contain frame and refuses letterbox clicks', () => {
    const rect = { left: 10, top: 20, width: 1000, height: 1000 };
    expect(browserRemotePoint(510, 520, rect, { width: 1920, height: 1080 })).toEqual({ x: 960, y: 540 });
    expect(browserRemotePoint(510, 30, rect, { width: 1920, height: 1080 })).toBeNull();
    expect(browserRemotePoint(1010, 801.25, rect, { width: 1920, height: 1080 })).toEqual({ x: 1919, y: 1079 });
    expect(browserRemotePoint(NaN, 10, rect, { width: 1920, height: 1080 })).toBeNull();
    expect(browserRemotePoint(10, 10, rect, { width: 0, height: 1080 })).toBeNull();
  });
  it.each([[0, 'left'], [1, 'middle'], [2, 'right'], [3, 'x1'], [4, 'x2'], [5, null]] as const)('maps browser button %s to %s', (button, expected) => {
    expect(browserPointerButton(button)).toBe(expected);
  });
  it.each([
    ['KeyA', 'a', 0x41], ['Digit7', '7', 0x37], ['ControlLeft', 'Control', 0x11],
    ['ShiftRight', 'Shift', 0x10], ['AltLeft', 'Alt', 0x12], ['MetaLeft', 'Meta', 0x5b],
    ['Enter', 'Enter', 0x0d], ['ArrowLeft', 'ArrowLeft', 0x25], ['Numpad5', '5', 0x65],
    ['NumpadAdd', '+', 0x6b], ['F12', 'F12', 0x7b], ['Slash', '/', 0xbf], ['Unidentified', 'Dead', null],
  ] as const)('maps physical %s without relying on layout text', (code, key, expected) => {
    expect(browserVirtualKey({ code, key })).toBe(expected);
  });
  it('normalizes pixel, line, and page wheels with bounded integer deltas', () => {
    expect(browserWheelDelta(120, 0, 540)).toBe(-120);
    expect(browserWheelDelta(-3, 1, 540)).toBe(48);
    expect(browserWheelDelta(1, 2, 540)).toBe(-540);
    expect(browserWheelDelta(Infinity, 0, 540)).toBe(0);
    expect(browserWheelDelta(100000, 0, 540)).toBe(-32767);
  });
});
