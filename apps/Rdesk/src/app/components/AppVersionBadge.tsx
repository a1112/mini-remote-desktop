interface AppVersionBadgeProps {
  collapsed: boolean;
  isDark: boolean;
}

const appVersion = `v${__APP_VERSION__}`;

export function AppVersionBadge({ collapsed, isDark }: AppVersionBadgeProps) {
  return (
    <div
      className={`shrink-0 border-t px-2 py-2 ${
        isDark ? "border-gray-700 text-gray-500" : "border-gray-300/40 text-gray-500"
      }`}
    >
      <div
        title={`Rdesk ${appVersion}`}
        className={`rounded-md font-mono tracking-tight ${
          collapsed ? "text-center text-[9px]" : "px-1 text-[10px]"
        } ${isDark ? "bg-black/10" : "bg-white/35"}`}
      >
        {appVersion}
      </div>
    </div>
  );
}
