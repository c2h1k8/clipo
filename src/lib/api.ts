import { invoke, convertFileSrc } from "@tauri-apps/api/core";

export type Item = {
  id: number;
  kind: "text" | "image" | "file";
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
  /** 「保存」に入っている（履歴の上限で消えず、履歴一覧には出ない） */
  saved: boolean;
  title: string | null;
  /** 呼び出し用の短い名前 */
  keyword: string | null;
  use_count: number;
  /** ファイルカードで、元のファイルが見つからないものがある */
  missing: boolean;
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
  /** 背景をすりガラスにする（オフなら後ろがそのまま透ける） */
  panel_blur: boolean;
  card_size: CardSize;
  excluded_apps: string[];
  group_by_date: boolean;
  multi_separator: string;
  launch_at_login: boolean;
};

export type CardSize = "small" | "medium" | "large";
/** カードの大きさ（Panel のレイアウト計算と CSS で使う） */
export const CARD_SIZES: Record<CardSize, { w: number; h: number; lines: number; label: string }> = {
  small: { w: 196, h: 172, lines: 4, label: "小" },
  medium: { w: 236, h: 204, lines: 6, label: "中" },
  large: { w: 276, h: 240, lines: 8, label: "大" },
};

/** 種類の絞り込み */
export type KindFilter = "text" | "link" | "image" | "file";
export const KIND_LABEL: Record<KindFilter, string> = { text: "テキスト", link: "リンク", image: "画像", file: "ファイル" };
export const KIND_ICON: Record<KindFilter, string> = { text: "text", link: "link", image: "image", file: "folder" };

/** URL だけのテキストか（バックエンドの判定と同じ） */
export const isLink = (t: string) => /^https?:\/\/\S+$/.test(t.trim());

/** 形を変えて貼る */
export const TRANSFORMS: [string, string][] = [
  ["half", "英数字を半角に"],
  ["full", "英数字を全角に"],
  ["oneline", "改行を消して1行に"],
  ["trim", "行の前後の空白を消す"],
  ["upper", "大文字に"],
  ["lower", "小文字に"],
];

/** 履歴 / 保存（tag でタグ絞り込み） */
export type Tab = { kind: "history" } | { kind: "saved"; tag: number | null };

export const api = {
  listItems: (tab: Tab, sourceId: string | null, query: string, range: [number, number] | null = null, kind: KindFilter | null = null) =>
    invoke<Item[]>("list_items", {
      filter: {
        kind,
        tab: tab.kind,
        tag_id: tab.kind === "saved" ? tab.tag : null,
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
  updateTag: (id: number, name: string, color: string) => invoke<void>("update_tag", { id, name, color }),
  deleteTag: (id: number) => invoke<void>("delete_tag", { id }),
  setTag: (itemIds: number[], tagId: number, on: boolean) => invoke<void>("set_tag", { itemIds, tagId, on }),
  toggleTag: (itemId: number, tagId: number) => invoke<boolean>("toggle_tag", { itemId, tagId }),
  setSaved: (ids: number[], saved: boolean) => invoke<void>("set_saved", { ids, saved }),
  deleteItems: (ids: number[]) => invoke<void>("delete_items", { ids }),
  clearHistory: () => invoke<void>("clear_history"),
  /** text は画像なら null */
  saveItem: (id: number | null, title: string, keyword: string, text: string | null) =>
    invoke<number>("save_item", { id, title, keyword, text }),
  /** asPath: ファイルをフルパスの文字として貼る */
  paste: (ids: number[], copyOnly = false, asPath = false, transform: string | null = null) =>
    invoke<void>("paste_items", { ids, copyOnly, vars: pasteVars(), asPath, transform }),
  /** 続けて貼る: 1 件目をクリップボードへ。⌘V / Ctrl+V のたびに次へ。件数を返す */
  startQueue: (ids: number[]) => invoke<number>("start_queue", { ids, vars: pasteVars() }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  getPaused: () => invoke<boolean>("get_paused"),
  /** 保存したテキストをダウンロードフォルダに書き出し、パスを返す */
  exportSaved: () => invoke<string>("export_saved"),
  importSaved: (json: string) => invoke<[number, number]>("import_saved", { json }),
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

/** 差し込み記号 {日付} {時刻} の値（ローカル時刻） */
function pasteVars() {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return {
    date: `${d.getFullYear()}/${p(d.getMonth() + 1)}/${p(d.getDate())}`,
    time: `${p(d.getHours())}:${p(d.getMinutes())}`,
  };
}

/** 保存したテキストで使える差し込み記号 */
export const PLACEHOLDERS = ["{日付}", "{時刻}", "{クリップボード}", "{カーソル}"];

/** ファイルカードの中身（改行区切りのパス）→ ファイル名の一覧 */
export const fileNames = (paths: string) =>
  paths.split("\n").filter(Boolean).map((p) => p.split(/[\\/]/).filter(Boolean).pop() ?? p);

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
  if (acc.startsWith("DoubleTap+")) {
    const mod = acc.slice("DoubleTap+".length);
    const name = isMac
      ? ({ Control: "⌃", Shift: "⇧", Alt: "⌥", Command: "⌘" } as Record<string, string>)[mod] ?? mod
      : ({ Control: "Ctrl", Command: "Win" } as Record<string, string>)[mod] ?? mod;
    return `${name} を2回`;
  }
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
