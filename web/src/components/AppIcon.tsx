import type { SVGProps } from "react";

export type AppIconName =
  | "overview"
  | "alert"
  | "node"
  | "theme"
  | "refresh"
  | "settings"
  | "activity"
  | "cpu"
  | "memory"
  | "offline"
  | "warning"
  | "check"
  | "timeline"
  | "search"
  | "download"
  | "upload"
  | "grid"
  | "list"
  | "monitor"
  | "server"
  | "plus"
  | "loader"
  | "save"
  | "copy"
  | "external"
  | "eye"
  | "eye-off"
  | "lock"
  | "shield"
  | "sparkle"
  | "back"
  | "edit"
  | "trash"
  | "disk"
  | "network"
  | "temperature"
  | "box"
  | "sort"
  | "menu"
  | "close"
  | "logout"
  | "chevron-right";

interface AppIconProps extends Omit<SVGProps<SVGSVGElement>, "name"> {
  name: AppIconName;
  size?: number | string;
}

/** The selected D icon language: compact modules, short lines, and steady geometry. */
export function AppIcon({ name, size = 18, strokeWidth = 1.8, ...props }: AppIconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={strokeWidth} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false" {...props}>
      {name === "overview" && <><rect x="3" y="3" width="8" height="8" rx="2" /><rect x="13" y="3" width="8" height="8" rx="2" /><rect x="3" y="13" width="8" height="8" rx="2" /><path d="M14 17h7" /></>}
      {name === "alert" && <><path d="M5 17h14M7 17v-5a5 5 0 0 1 10 0v5M10 20h4M12 3v2" /></>}
      {name === "node" && <><rect x="4" y="4" width="16" height="16" rx="4" /><path d="M8 16V9m4 7v-4m4 4V7" /></>}
      {name === "theme" && <path d="M4 16a8 8 0 1 0 8-12v8a4 4 0 0 1-4 4H4Z" />}
      {name === "refresh" && <path d="M20 11a8 8 0 0 0-14-4L3 10m0-5v5h5M4 13a8 8 0 0 0 14 4l3-3m0 5v-5h-5" />}
      {name === "settings" && <><path d="M4 7h10M18 7h2M4 12h2M10 12h10M4 17h10M18 17h2" /><circle cx="16" cy="7" r="2" /><circle cx="8" cy="12" r="2" /><circle cx="16" cy="17" r="2" /></>}
      {name === "activity" && <><path d="M3 13h5l2-5 3 9 2-5h6" /><circle cx="3" cy="13" r="1.5" fill="currentColor" stroke="none" /></>}
      {name === "cpu" && <><rect x="5" y="5" width="14" height="14" rx="3" /><path d="M9 9h6v6H9zM9 2v3m6-3v3M9 19v3m6-3v3M2 9h3m-3 6h3m14-6h3m-3 6h3" /></>}
      {name === "memory" && <><rect x="3" y="7" width="18" height="10" rx="2" /><path d="M7 7V4m4 3V4m4 3V4m4 3V4M7 17v3m4-3v3m4-3v3m4-3v3M7 10h10M7 14h7" /></>}
      {name === "offline" && <><rect x="4" y="4" width="16" height="16" rx="4" /><path d="m7 7 10 10M8 16v-6m4 6v-3m4 3v-7" /></>}
      {name === "warning" && <><path d="m12 4 8 15H4L12 4Z" /><path d="M12 9v4m0 3v.1" /></>}
      {name === "check" && <><circle cx="12" cy="12" r="8" /><path d="m8 12 3 3 5-6" /></>}
      {name === "timeline" && <><circle cx="6" cy="12" r="2" /><path d="M8 12h10m0 0-3-3m3 3-3 3" /></>}
      {name === "search" && <><circle cx="10.5" cy="10.5" r="6" /><path d="m15 15 5 5" /></>}
      {name === "download" && <><path d="M12 4v11m0 0 4-4m-4 4-4-4M5 20h14" /></>}
      {name === "upload" && <><path d="M12 20V9m0 0 4 4m-4-4-4 4M5 4h14" /></>}
      {name === "grid" && <><rect x="4" y="4" width="6" height="6" rx="1.5" /><rect x="14" y="4" width="6" height="6" rx="1.5" /><rect x="4" y="14" width="6" height="6" rx="1.5" /><rect x="14" y="14" width="6" height="6" rx="1.5" /></>}
      {name === "list" && <><path d="M9 6h11M9 12h11M9 18h11" /><path d="M4 6h.1M4 12h.1M4 18h.1" strokeWidth="2.4" /></>}
      {name === "monitor" && <><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 20h8M12 16v4" /></>}
      {name === "server" && <><rect x="4" y="4" width="16" height="6" rx="2" /><rect x="4" y="14" width="16" height="6" rx="2" /><path d="M8 7h.1M8 17h.1M12 7h5M12 17h5" strokeWidth="2.2" /></>}
      {name === "plus" && <path d="M12 5v14M5 12h14" />}
      {name === "loader" && <path d="M20 12a8 8 0 1 1-2.3-5.7" />}
      {name === "save" && <><path d="M5 4h12l2 2v14H5z" /><path d="M8 4v6h8V4M8 20v-6h8v6" /></>}
      {name === "copy" && <><rect x="8" y="8" width="12" height="12" rx="2" /><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" /></>}
      {name === "external" && <><path d="M14 4h6v6M20 4l-9 9" /><path d="M18 13v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h5" /></>}
      {name === "eye" && <><path d="M3 12s3-6 9-6 9 6 9 6-3 6-9 6-9-6-9-6Z" /><circle cx="12" cy="12" r="2.5" /></>}
      {name === "eye-off" && <><path d="m4 4 16 16M10.6 6.2A9.7 9.7 0 0 1 12 6c6 0 9 6 9 6a16 16 0 0 1-3 3.6M6.5 6.9C4.2 8.5 3 12 3 12s3 6 9 6c1 0 1.9-.2 2.7-.5" /></>}
      {name === "lock" && <><rect x="5" y="10" width="14" height="10" rx="2" /><path d="M8 10V7a4 4 0 0 1 8 0v3" /></>}
      {name === "shield" && <path d="M12 3 19 6v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6l7-3Z" />}
      {name === "sparkle" && <><path d="m12 3 1.4 5.6L19 10l-5.6 1.4L12 17l-1.4-5.6L5 10l5.6-1.4L12 3ZM19 16l.6 2.4L22 19l-2.4.6L19 22l-.6-2.4L16 19l2.4-.6L19 16Z" /></>}
      {name === "menu" && <path d="M4 7h16M4 12h16M4 17h16" />}
      {name === "close" && <path d="m6 6 12 12M18 6 6 18" />}
      {name === "logout" && <><path d="M13 5h5v14h-5M10 8l4 4-4 4M14 12H3" /></>}
      {name === "chevron-right" && <path d="m9 5 7 7-7 7" />}
      {name === "back" && <><path d="M19 12H5m6-6-6 6 6 6" /></>}
      {name === "edit" && <><path d="m5 16-.8 3.8L8 19l10.5-10.5a2.1 2.1 0 0 0-3-3L5 16Z" /><path d="m13.5 7.5 3 3" /></>}
      {name === "trash" && <><path d="M5 7h14M10 11v5m4-5v5M9 7V4h6v3m-9 0 1 13h10l1-13" /></>}
      {name === "disk" && <><path d="M5 4h14l1 2v14H4V6l1-2Z" /><path d="M8 4v6h8V4M8 20v-5h8v5" /></>}
      {name === "network" && <><path d="M4 7h16M4 17h16M8 7v10m8-10v10" /><circle cx="4" cy="7" r="1.5" fill="currentColor" stroke="none" /><circle cx="20" cy="17" r="1.5" fill="currentColor" stroke="none" /></>}
      {name === "temperature" && <><path d="M9 14V6a3 3 0 0 1 6 0v8a5 5 0 1 1-6 0Z" /><path d="M12 6v9" /></>}
      {name === "box" && <><path d="m12 3 8 4.5v9L12 21l-8-4.5v-9L12 3Z" /><path d="m12 3 8 4.5-8 4.5-8-4.5L12 3ZM12 12v9" /></>}
      {name === "sort" && <><path d="M6 5v14m0 0-3-3m3 3 3-3M18 19V5m0 0-3 3m3-3 3 3" /></>}
    </svg>
  );
}
