import { invoke, convertFileSrc } from "@tauri-apps/api/core";

export type Item = {
  id: number;
  kind: "text" | "image";
  preview: string;
  char_count: number;
  image_path: string | null;
  thumb_path: string | null;
  width: number | null;
  height: number | null;
  source_name: string | null;
  source_id: string | null;
  source_icon: string | null;
  created_at: number;
  used_at: number;
  favorite: boolean;
  snippet: boolean;
  title: string | null;
  translated: string | null;
  tags: number[];
};

export type Tag = { id: number; name: string; color: string };
export type SourceApp = { source_id: string; source_name: string; source_icon: string | null; count: number };

export type Settings = {
  panel_shortcut: string;
  ocr_shortcut: string;
  screenshot_enabled: boolean;
  watch_screenshots: boolean;
  screenshot_shortcut: string;
  translate_provider: "google" | "deepl";
  deepl_api_key: string;
  translate_target: string;
  max_history: number;
  panel_rows: number;
  panel_opacity: number;
  group_by_date: boolean;
  multi_separator: string;
  launch_at_login: boolean;
};

export type Tab = { kind: "all" } | { kind: "favorite" } | { kind: "snippet" } | { kind: "tag"; id: number };

export const api = {
  listItems: (tab: Tab, sourceId: string | null, query: string, range: [number, number] | null = null) =>
    invoke<Item[]>("list_items", {
      filter: {
        tab: tab.kind,
        tag_id: tab.kind === "tag" ? tab.id : null,
        source_id: sourceId,
        query,
        limit: 300,
        since: range?.[0] ?? null,
        until: range?.[1] ?? null,
      },
    }),
  itemText: (id: number) => invoke<string | null>("item_text", { id }),
  sourceApps: () => invoke<SourceApp[]>("source_apps"),
  listTags: () => invoke<Tag[]>("list_tags"),
  createTag: (name: string, color: string) => invoke<Tag>("create_tag", { name, color }),
  deleteTag: (id: number) => invoke<void>("delete_tag", { id }),
  setTag: (itemIds: number[], tagId: number, on: boolean) => invoke<void>("set_tag", { itemIds, tagId, on }),
  toggleTag: (itemId: number, tagId: number) => invoke<boolean>("toggle_tag", { itemId, tagId }),
  setFavorite: (id: number, favorite: boolean) => invoke<void>("set_favorite", { id, favorite }),
  deleteItems: (ids: number[]) => invoke<void>("delete_items", { ids }),
  clearHistory: () => invoke<void>("clear_history"),
  saveSnippet: (id: number | null, title: string, text: string) =>
    invoke<number>("save_snippet", { id, title, text }),
  paste: (ids: number[], copyOnly = false) => invoke<void>("paste_items", { ids, copyOnly }),
  translate: (id: number) => invoke<string>("translate_item", { id }),
  revertTranslation: (id: number) => invoke<void>("revert_translation", { id }),
  ocrCapture: () => invoke<void>("ocr_capture"),
  ocrItem: (id: number) => invoke<void>("ocr_item", { id }),
  screenshot: () => invoke<void>("take_screenshot"),
  hide: () => invoke<void>("hide_panel"),
  resizePanel: (height: number) => invoke<void>("resize_panel", { height }),
  openSettings: () => invoke<void>("open_settings"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<string[]>("save_settings", { settings }),
  pauseShortcuts: (paused: boolean) => invoke<void>("pause_shortcuts", { paused }),
  accessibilityStatus: () => invoke<boolean>("accessibility_status"),
  openAccessibility: () => invoke<void>("open_accessibility"),
  platform: () => invoke<string>("platform_name"),
};

export const fileSrc = (p: string | null) => (p ? convertFileSrc(p) : "");

export const TAG_COLORS = ["#ef4444", "#f59e0b", "#10b981", "#3b82f6", "#8b5cf6", "#ec4899", "#14b8a6", "#64748b"];

export function timeAgo(ms: number, now: number): string {
  const s = Math.max(0, Math.floor((now - ms) / 1000));
  if (s < 60) return `${s}秒前`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}分前`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}時間前`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}日前`;
  return new Date(ms).toLocaleDateString("ja-JP");
}

export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent);

/** "CommandOrControl+Shift+V" → "⌘⇧V" / "Ctrl+Shift+V" */
export function prettyShortcut(acc: string): string {
  if (!acc) return "未設定";
  const parts = acc.split("+");
  if (isMac) {
    const map: Record<string, string> = {
      CommandOrControl: "⌘", Command: "⌘", Cmd: "⌘", Super: "⌘", Meta: "⌘",
      Control: "⌃", Ctrl: "⌃", Alt: "⌥", Option: "⌥", Shift: "⇧",
    };
    return parts.map((p) => map[p] ?? p).join("");
  }
  const map: Record<string, string> = { CommandOrControl: "Ctrl", Control: "Ctrl", Super: "Win", Meta: "Win" };
  return parts.map((p) => map[p] ?? p).join("+");
}

export type DateRange = "all" | "today" | "yesterday" | "week" | "month";

export const DATE_RANGE_LABEL: Record<DateRange, string> = {
  all: "すべての期間",
  today: "今日",
  yesterday: "昨日",
  week: "過去7日",
  month: "過去30日",
};

function startOfDay(t: number): number {
  const d = new Date(t);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** 期間 → [since, until) (ms)。all は null */
export function dateRangeBounds(r: DateRange, now = Date.now()): [number, number] | null {
  const today = startOfDay(now);
  const DAY = 86_400_000;
  switch (r) {
    case "today": return [today, today + DAY];
    case "yesterday": return [today - DAY, today];
    case "week": return [today - 6 * DAY, today + DAY];
    case "month": return [today - 29 * DAY, today + DAY];
    default: return null;
  }
}

/** 日付の区切りに表示する見出し */
export function dayLabel(t: number, now = Date.now()): string {
  const diff = Math.round((startOfDay(now) - startOfDay(t)) / 86_400_000);
  if (diff === 0) return "今日";
  if (diff === 1) return "昨日";
  const d = new Date(t);
  const w = "日月火水木金土"[d.getDay()];
  if (diff < 7) return `${d.getMonth() + 1}/${d.getDate()}（${w}）`;
  if (d.getFullYear() === new Date(now).getFullYear()) return `${d.getMonth() + 1}月${d.getDate()}日`;
  return `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日`;
}

export function dayKey(t: number): number {
  return startOfDay(t);
}
