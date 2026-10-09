import { useEffect, useRef } from 'react';
import { Settings, X } from 'lucide-react';
import { useTheme } from './ThemeContext';
import { SettingsContent } from './SettingsContent';

interface SettingsModalProps { open: boolean; onClose: () => void }

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const { isDark } = useTheme();
  const panel = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    panel.current?.focus();
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); close.current(); }
      if (event.key !== 'Tab' || !panel.current) return;
      const items = Array.from(panel.current.querySelectorAll<HTMLElement>('button:not(:disabled), select:not(:disabled), input:not(:disabled), a[href], [tabindex="0"]'));
      const first = items[0];
      const last = items[items.length - 1];
      if (!first || !last) { event.preventDefault(); panel.current.focus(); return; }
      if (event.shiftKey && (document.activeElement === first || document.activeElement === panel.current)) {
        event.preventDefault(); last.focus();
      } else if (!event.shiftKey && (document.activeElement === last || document.activeElement === panel.current)) {
        event.preventDefault(); first.focus();
      }
    };
    document.addEventListener('keydown', handleKey);
    return () => { document.removeEventListener('keydown', handleKey); previous?.focus(); };
  }, [open]);

  // Unmount content on close so a previous opening cannot write into the next one.
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-3 sm:p-6">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div ref={panel} role="dialog" aria-modal="true" aria-labelledby="settings-dialog-title" tabIndex={-1}
        className={`relative flex flex-col overflow-hidden rounded-xl border shadow-2xl outline-none ${isDark ? 'bg-[#1e1e1e] border-gray-700 text-gray-100' : 'bg-white border-gray-200 text-gray-900'}`}
        style={{ width: 880, maxWidth: '100%', height: 580, maxHeight: 'calc(100dvh - 32px)' }}>
        <header className={`flex shrink-0 items-center gap-3 border-b px-5 py-3.5 ${isDark ? 'border-gray-700 bg-[#222]' : 'border-gray-200 bg-gray-50'}`}>
          <Settings className="h-4 w-4 text-gray-500" />
          <h2 id="settings-dialog-title" className="flex-1 text-sm font-semibold">设置</h2>
          <button type="button" aria-label="关闭设置" onClick={onClose} className="rounded-lg p-1.5 text-gray-400 hover:bg-gray-500/15"><X className="h-4 w-4" /></button>
        </header>
        <SettingsContent />
      </div>
    </div>
  );
}
