/**
 * Shared theme contract for the PingLake UI.
 *
 * The preference includes `system`, while the other values are concrete
 * themes. Keeping the preference separate from the resolved theme lets the
 * app follow OS appearance without losing the user's explicit choice.
 */
export type ThemePreference = "system" | "obsidian" | "porcelain" | "lagoon" | "amber";

export type ResolvedTheme = Exclude<ThemePreference, "system">;

export type ThemeColorScheme = "light" | "dark";

export interface ThemeDefinition {
  id: ResolvedTheme;
  label: string;
  description: string;
  colorScheme: ThemeColorScheme;
  /** Primary brand color used by controls, focus rings, and charts. */
  accent: string;
  /** The browser chrome color used by the theme-color meta tag. */
  themeColor: string;
  /** Small swatches used by the theme picker preview. */
  swatches: readonly [string, string, string];
}
export interface ThemePreferenceOption {
  id: ThemePreference;
  label: string;
  description: string;
  swatches?: readonly [string, string, string];
}

export const THEME_STORAGE_KEY = "pinglake.theme";

export const THEME_DEFINITIONS: Record<ResolvedTheme, ThemeDefinition> = {
  obsidian: {
    id: "obsidian",
    label: "Obsidian 曜石",
    description: "深石墨画布与冷紫蓝高光，适合低光环境。",
    colorScheme: "dark",
    accent: "#8B8EFF",
    themeColor: "#0B0D12",
    swatches: ["#0B0D12", "#141720", "#8B8EFF"],
  },
  porcelain: {
    id: "porcelain",
    label: "Porcelain 云瓷",
    description: "微暖浅灰背景与柔和蓝色，清晰而耐看。",
    colorScheme: "light",
    accent: "#5570D9",
    themeColor: "#F4F5F8",
    swatches: ["#F4F5F8", "#FFFFFF", "#5570D9"],
  },
  lagoon: {
    id: "lagoon",
    label: "Lagoon 深海",
    description: "深蓝绿层次与青绿色信号，沉浸且专注。",
    colorScheme: "dark",
    accent: "#5AD6C5",
    themeColor: "#09191C",
    swatches: ["#09191C", "#11262C", "#5AD6C5"],
  },
  amber: {
    id: "amber",
    label: "Amber 琥珀",
    description: "温暖米白画布与古铜橙色，柔和而有质感。",
    colorScheme: "light",
    accent: "#B86A36",
    themeColor: "#F7F2EA",
    swatches: ["#F7F2EA", "#FFFDFC", "#B86A36"],
  },
};

export const THEME_PREFERENCE_OPTIONS: readonly ThemePreferenceOption[] = [
  {
    id: "system",
    label: "跟随系统",
    description: "根据操作系统的浅色或深色设置自动切换。",
  },
  ...Object.values(THEME_DEFINITIONS).map(({ id, label, description, swatches }) => ({
    id,
    label,
    description,
    swatches,
  })),
];

const LEGACY_THEME_MIGRATIONS: Readonly<Record<string, ThemePreference>> = {
  light: "porcelain",
  dark: "obsidian",
  midnight: "lagoon",
  circuit: "lagoon",
};

export function isResolvedTheme(value: unknown): value is ResolvedTheme {
  return value === "obsidian" || value === "porcelain" || value === "lagoon" || value === "amber";
}

export function isThemePreference(value: unknown): value is ThemePreference {
  return value === "system" || isResolvedTheme(value);
}

/** Convert values from pre-redesign builds to the new preference contract. */
export function migrateThemePreference(value: unknown): ThemePreference {
  if (typeof value !== "string") return "system";
  if (isThemePreference(value)) return value;
  return LEGACY_THEME_MIGRATIONS[value] ?? "system";
}

/** Read and, when needed, rewrite a stored preference using the current key. */
export function readThemePreference(storage?: Pick<Storage, "getItem" | "setItem">): ThemePreference {
  let target = storage;
  if (!target && typeof window !== "undefined") {
    try {
      target = window.localStorage;
    } catch {
      target = undefined;
    }
  }

  let stored: string | null = null;
  try {
    stored = target?.getItem(THEME_STORAGE_KEY) ?? null;
  } catch {
    stored = null;
  }

  const preference = migrateThemePreference(stored);
  if (target && stored !== preference) {
    try {
      target.setItem(THEME_STORAGE_KEY, preference);
    } catch {
      // Storage can be unavailable in private browsing or sandboxed frames.
    }
  }
  return preference;
}

export function systemPrefersDark(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)").matches
    : false;
}

/** Resolve `system` to the concrete theme that should be rendered right now. */
export function resolveThemePreference(preference: ThemePreference, prefersDark = systemPrefersDark()): ResolvedTheme {
  if (preference === "system") return prefersDark ? "obsidian" : "porcelain";
  return preference;
}

/**
 * Apply the resolved theme to the document and keep browser chrome in sync.
 * This is safe to call from an effect and is a no-op during SSR.
 */
export function applyTheme(
  preference: ThemePreference,
  options: { prefersDark?: boolean; persist?: boolean; storage?: Pick<Storage, "setItem"> } = {},
): ResolvedTheme {
  const resolved = resolveThemePreference(preference, options.prefersDark);
  const definition = THEME_DEFINITIONS[resolved];

  if (typeof document !== "undefined") {
    const root = document.documentElement;
    root.dataset.theme = resolved;
    root.style.colorScheme = definition.colorScheme;
    document.querySelector<HTMLMetaElement>('meta[name="theme-color"]')?.setAttribute("content", definition.themeColor);
  }

  if (options.persist !== false) {
    let storage = options.storage;
    if (!storage && typeof window !== "undefined") {
      try {
        storage = window.localStorage;
      } catch {
        storage = undefined;
      }
    }
    try {
      storage?.setItem(THEME_STORAGE_KEY, preference);
    } catch {
      // Storage is optional; theme application should still complete.
    }
  }

  return resolved;
}
