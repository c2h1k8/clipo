<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    api, fileSrc, fileNames, isLink, isMac, TAG_COLORS, PLACEHOLDERS, DATE_RANGE_LABEL, dateRangeBounds, dayKey, dayLabel,
    CARD_SIZES, KIND_LABEL, KIND_ICON, TRANSFORMS,
    type CardSize, type DateRange, type Item, type KindFilter, type SourceApp, type Tab, type Tag,
  } from "./api";
  import Card from "./Card.svelte";
  import Icon from "./Icon.svelte";
  import Menu, { type MenuEntry } from "./Menu.svelte";

  const MOD = isMac ? "⌘" : "Ctrl+";
  const K = isMac
    ? { enter: "⏎", shift: "⇧", del: "⌫", alt: "⌥" }
    : { enter: "Enter", shift: "Shift+", del: "Del", alt: "Alt+" };

  let items = $state<Item[]>([]);
  let tags = $state<Tag[]>([]);
  let apps = $state<SourceApp[]>([]);
  let tab = $state<Tab>({ kind: "history" });
  let sourceId = $state<string | null>(null);
  let query = $state("");
  let dateRange = $state<DateRange>("all");
  let groupByDate = $state(true);
  let selected = $state<number[]>([]); // 選んだ順
  let focusIdx = $state(0);
  let anchorIdx = $state(0);
  let now = $state(Date.now());
  let busy = $state<Record<number, boolean>>({});
  let toast = $state<{ text: string; action?: { label: string; run: () => void } } | null>(null);
  let addingTag = $state(false);
  let newTagName = $state("");
  /** 名前を変更中のタグ */
  let renaming = $state<{ id: number; name: string } | null>(null);
  let renameInput = $state<HTMLInputElement | null>(null);
  /** 開いているメニュー（右クリック/コピー元/タグ） */
  let menu = $state<{ kind: "card" | "app" | "tag" | "tagpill" | "tagedit" | "date" | "moretags" | "kind" | "transform"; x: number; y: number; itemId?: number; tagId?: number } | null>(null);
  let menuRef = $state<Menu | null>(null);
  let help = $state(false);
  /** 種類の絞り込み */
  let kindFilter = $state<KindFilter | null>(null);
  let kindBtn = $state<HTMLButtonElement | null>(null);
  let cardSize = $state<CardSize>("medium");
  /** 記録を一時停止中 */
  let paused = $state(false);
  /** Space で大きく表示 */
  let preview = $state(false);
  let previewText = $state<string | null>(null);
  let appBtn: HTMLButtonElement;
  let dateBtn = $state<HTMLButtonElement | null>(null);
  let tagInput = $state<HTMLInputElement | null>(null);
  /** 保存の編集。text が null なら画像（タイトルと短い名前だけ） */
  let editor = $state<{ id: number | null; title: string; keyword: string; text: string | null } | null>(null);
  let editorText = $state<HTMLTextAreaElement | null>(null);
  let screenshotEnabled = $state(true);
  /** 設定の最大段数 */
  let maxRows = $state(1);
  /** 実際の段数：1段に収まらないときだけ、最大段数まで増やす */
  let rows = $state(1);
  let panelWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1200);
  // レイアウト計算（CSS と同じ値）
  const GAP = 12, PAD_X = 32;
  const CARD_W = $derived(CARD_SIZES[cardSize].w);
  const CARD_H = $derived(CARD_SIZES[cardSize].h);
  /** バー 48 + ヒント 26 + 一覧の上下余白 22 */
  const CHROME_H = 96;
  const showDays = $derived(groupByDate && tab.kind === "history");
  /** 保存タブでは先頭に「新しく保存」カードが入る */
  const lead = $derived(tab.kind === "saved" ? 1 : 0);
  /**
   * 1段: 横に 1 列。
   * 多段: 「列数×段数」を 1 ページとし、ページ内は左上から右へ並べて折り返す。ページは横に並ぶ。
   * 1段に収まる枚数なら段数を増やさない（設定は最大段数）
   */
  const layout = $derived.by(() => {
    const count = items.length + lead;
    const cols = Math.max(1, Math.floor((panelWidth - PAD_X + GAP) / (CARD_W + GAP)));
    const r = maxRows <= 1 || count <= cols ? 1 : Math.min(maxRows, Math.ceil(count / cols));
    return { rows: r, cols, perPage: r * cols, height: CHROME_H + r * CARD_H + (r - 1) * GAP };
  });
  const paged = $derived(layout.rows > 1);
  /** ページごとのセル（-1 は「新しく保存」カード） */
  const pages = $derived.by(() => {
    const cells = [...(lead ? [-1] : []), ...items.map((_, i) => i)];
    const out: number[][] = [];
    for (let i = 0; i < cells.length; i += layout.perPage) out.push(cells.slice(i, i + layout.perPage));
    return out;
  });
  $effect(() => {
    rows = layout.rows;
    api.resizePanel(layout.height);
  });
  /** 背景の不透明度 0〜100（設定） */
  let opacity = $state(30);

  let listEl: HTMLDivElement;
  let searchEl: HTMLInputElement;
  let toastTimer: ReturnType<typeof setTimeout>;

  const currentApp = $derived(apps.find((a) => a.source_id === sourceId) ?? null);
  const selectedSet = $derived(new Set(selected));

  function showToast(text: string, action?: { label: string; run: () => void }, ms = 3500) {
    clearTimeout(toastTimer);
    toast = { text, action };
    toastTimer = setTimeout(() => (toast = null), ms);
  }

  async function reload(keepSelection = true) {
    const [it, tg, ap] = await Promise.all([api.listItems(tab, sourceId, query, dateRangeBounds(dateRange), kindFilter), api.listTags(), api.sourceApps()]);
    items = it;
    tags = tg;
    apps = ap;
    if (keepSelection) {
      const ids = new Set(it.map((i) => i.id));
      selected = selected.filter((id) => ids.has(id));
      focusIdx = Math.min(focusIdx, Math.max(0, it.length - 1));
    } else {
      focusIdx = 0;
      anchorIdx = 0;
      selected = it[0] ? [it[0].id] : [];
    }
  }

  function setTab(t: Tab) {
    tab = t;
    reload(false);
  }

  function setSource(id: string | null) {
    sourceId = id;
    reload(false);
  }

  async function scrollToFocus(align: "nearest" | "start" | "end" = "nearest") {
    await tick();
    if (paged) {
      // ページごとにスクロール
      const page = Math.floor((focusIdx + lead) / layout.perPage);
      listEl?.querySelector<HTMLElement>(`[data-page="${page}"]`)?.scrollIntoView({ inline: "start", block: "nearest", behavior: "smooth" });
      return;
    }
    listEl?.querySelector<HTMLElement>(`[data-index="${focusIdx}"]`)?.scrollIntoView({ block: "nearest", inline: align, behavior: "smooth" });
  }

  /** ページ式での位置 → カード番号（範囲外は端に寄せる） */
  function cellToIndex(cell: number): number {
    return Math.max(0, Math.min(items.length - 1, cell - lead));
  }

  /** 上下の段へ。ページの端では隣のページの同じ列へ */
  function verticalTarget(from: number, dir: 1 | -1): number {
    const { cols, perPage, rows: r } = layout;
    const cell = from + lead;
    const page = Math.floor(cell / perPage);
    const pos = cell % perPage;
    const row = Math.floor(pos / cols);
    const col = pos % cols;
    let target: number;
    if (dir > 0) target = row < r - 1 ? cell + cols : (page + 1) * perPage + col;
    else target = row > 0 ? cell - cols : (page - 1) * perPage + (r - 1) * cols + col;
    if (target < lead || target - lead > items.length - 1) {
      // 次の段が途中までしかない場合は最後のカードへ、それもなければ動かない
      const sameDirExists = dir > 0 ? cell + 1 <= items.length - 1 + lead && target > cell : false;
      return sameDirExists ? items.length - 1 : from;
    }
    return target - lead;
  }

  /** その日の最初のカードか（日付ラベル/区切りを出す位置） */
  function isDayStart(i: number): boolean {
    return showDays && (i === 0 || dayKey(items[i - 1].used_at) !== dayKey(items[i].used_at));
  }

  /** 前/次の日付の最初のカードへ */
  function jumpDay(dir: 1 | -1) {
    if (!items.length) return;
    const curDay = dayKey(items[focusIdx].used_at);
    if (dir > 0) {
      const i = items.findIndex((it, n) => n > focusIdx && dayKey(it.used_at) !== curDay);
      if (i >= 0) moveTo(i, false, "start");
    } else {
      // いまの日の先頭にいなければ先頭へ、先頭なら前の日の先頭へ
      let start = focusIdx;
      while (start > 0 && dayKey(items[start - 1].used_at) === curDay) start--;
      let target = start;
      if (start === focusIdx && start > 0) {
        const prevDay = dayKey(items[start - 1].used_at);
        target = start - 1;
        while (target > 0 && dayKey(items[target - 1].used_at) === prevDay) target--;
      }
      moveTo(target, false, "start");
    }
    showToast(dayLabel(items[focusIdx].used_at, now), undefined, 900);
  }

  /** いまの段の先頭/末尾 */
  function rowEdge(from: number, end: boolean): number {
    const { cols } = layout;
    const cell = from + lead;
    const start = cell - (cell % cols);
    return cellToIndex(end ? start + cols - 1 : start);
  }

  function selectRange(a: number, b: number) {
    const [lo, hi] = a < b ? [a, b] : [b, a];
    selected = items.slice(lo, hi + 1).map((i) => i.id);
  }

  /** 画面内に完全に見えているカードの範囲（1段のとき） */
  function visibleRange(): [number, number] {
    const box = listEl?.getBoundingClientRect();
    const cards = listEl ? [...listEl.querySelectorAll<HTMLElement>("[data-index]")] : [];
    const vis = cards
      .filter((c) => {
        const r = c.getBoundingClientRect();
        return box && r.left >= box.left - 1 && r.right <= box.right + 1;
      })
      .map((c) => Number(c.dataset.index));
    if (!vis.length) return [focusIdx, focusIdx];
    return [Math.min(...vis), Math.max(...vis)];
  }

  /** 1段のときの 1 ページ（見えている枚数） */
  function pageSize(): number {
    const [a, b] = visibleRange();
    return Math.max(1, b - a + 1);
  }

  function moveTo(idx: number, extend: boolean, align: "nearest" | "start" | "end" = "nearest") {
    if (!items.length) return;
    focusIdx = Math.max(0, Math.min(items.length - 1, idx));
    if (extend) selectRange(anchorIdx, focusIdx);
    else {
      anchorIdx = focusIdx;
      selected = [items[focusIdx].id];
    }
    scrollToFocus(align);
  }

  function move(delta: number, extend: boolean) {
    if (!items.length) return;
    focusIdx = Math.max(0, Math.min(items.length - 1, focusIdx + delta));
    if (extend) selectRange(anchorIdx, focusIdx);
    else {
      anchorIdx = focusIdx;
      selected = [items[focusIdx].id];
    }
    scrollToFocus();
  }

  function clickCard(e: MouseEvent, idx: number) {
    const id = items[idx].id;
    menu = null;
    if (e.shiftKey) {
      selectRange(anchorIdx, idx);
    } else if (e.metaKey || e.ctrlKey) {
      selected = selectedSet.has(id) ? selected.filter((x) => x !== id) : [...selected, id];
      anchorIdx = idx;
    } else {
      selected = [id];
      anchorIdx = idx;
    }
    focusIdx = idx;
  }

  function targetIds(): number[] {
    if (selected.length) return selected;
    return items[focusIdx] ? [items[focusIdx].id] : [];
  }

  async function paste(ids = targetIds(), copyOnly = false, asPath = false, transform: string | null = null) {
    if (!ids.length) return;
    menu = null;
    preview = false;
    try {
      await api.paste(ids, copyOnly, asPath, transform);
    } catch (err) {
      showToast(String(err), undefined, 5000);
    }
  }

  /** 保存する / 保存を外す。どちらもカードは今のタブから別のタブへ移る */
  async function toggleSaved(ids = targetIds()) {
    const list = items.filter((i) => ids.includes(i.id));
    if (!list.length) return;
    const on = !list.every((i) => i.saved);
    await api.setSaved(list.map((i) => i.id), on);
    menu = null;
    const n = list.length > 1 ? `${list.length}件を` : "";
    showToast(on ? `${n}保存しました` : `${n}保存から外して履歴に戻しました`, {
      label: on ? "保存を開く" : "履歴を開く",
      run: () => setTab(on ? { kind: "saved", tag: null } : { kind: "history" }),
    });
    await reload();
  }

  async function remove(ids = targetIds()) {
    if (!ids.length) return;
    await api.deleteItems(ids);
    menu = null;
    selected = [];
    await reload();
    if (items[focusIdx]) selected = [items[focusIdx].id];
  }

  async function runAction(item: Item) {
    if (busy[item.id] || item.kind === "file") return;
    if (item.kind === "text" && !item.translated && isLink(item.preview)) {
      await openLink(item);
      return;
    }
    busy[item.id] = true;
    try {
      if (item.kind === "image") {
        await api.ocrItem(item.id);
        showToast("文字起こしして履歴に追加しました");
      } else {
        await api.translate(item.id);
      }
      await reload();
    } catch (err) {
      showToast(String(err), { label: "設定", run: () => api.openSettings() }, 6000);
    } finally {
      busy[item.id] = false;
    }
  }

  async function revert(item: Item) {
    await api.revertTranslation(item.id);
    reload();
  }

  /** 新しいタグを作成。作成後に付けるカード（メニューから作ったときだけ） */
  let newTagTargets = $state<number[]>([]);

  function cancelNewTag() {
    addingTag = false;
    newTagName = "";
    newTagTargets = [];
  }

  async function commitNewTag() {
    const name = newTagName.trim();
    const targets = newTagTargets;
    cancelNewTag();
    if (!name) return;
    const existing = tags.find((t) => t.name === name);
    const t = existing ?? (await api.createTag(name, TAG_COLORS[tags.length % TAG_COLORS.length]));
    if (targets.length) {
      await api.setTag(targets, t.id, true);
      showToast(`${targets.length > 1 ? `${targets.length}件に` : ""}タグ「${t.name}」を付けて保存しました`);
    } else {
      showToast(existing ? `タグ「${t.name}」はすでにあります` : `タグ「${t.name}」を作成しました`);
    }
    await reload();
  }

  /** 対象カード（複数選択中ならその全部）。全部に付いていれば外し、そうでなければ全部に付ける */
  async function toggleTagOn(item: Item, tagId: number) {
    const ids = selectedSet.has(item.id) && selected.length > 1 ? selected : [item.id];
    const targets = items.filter((i) => ids.includes(i.id));
    const on = !targets.every((i) => i.tags.includes(tagId));
    await api.setTag(ids, tagId, on);
    const name = tags.find((t) => t.id === tagId)?.name ?? "";
    const saving = on && targets.some((i) => !i.saved);
    showToast(on ? `「${name}」を付けました${saving ? "（保存に入りました）" : ""}` : `「${name}」を外しました`, undefined, 1800);
    await reload();
  }

  /** forItem を渡すと、作成したタグをそのカード（と複数選択）に付ける */
  async function startNewTag(forItem?: Item) {
    if (forItem) {
      newTagTargets = selectedSet.has(forItem.id) && selected.length > 1 ? [...selected] : [forItem.id];
    } else {
      newTagTargets = [];
    }
    menu = null;
    addingTag = true;
    await tick();
    tagInput?.focus();
  }

  /** フォーカス中のカードの位置（キーボードでメニューを開くとき用） */
  function focusedCardPos(): { x: number; y: number } {
    const el = listEl?.querySelector<HTMLElement>(`[data-index="${focusIdx}"]`);
    const r = el?.getBoundingClientRect();
    return r ? { x: r.left + 24, y: r.top + 40 } : { x: 40, y: 60 };
  }

  function placeMenu(x: number, y: number, h = 300, w = 230) {
    return {
      x: Math.max(8, Math.min(x, window.innerWidth - w - 8)),
      y: Math.max(8, Math.min(y, window.innerHeight - h - 8)),
    };
  }

  function openCardMenu(idx = focusIdx, at?: { x: number; y: number }) {
    const item = items[idx];
    if (!item) return;
    if (!selectedSet.has(item.id)) {
      selected = [item.id];
      anchorIdx = idx;
    }
    focusIdx = idx;
    const p = at ?? focusedCardPos();
    menu = { kind: "card", itemId: item.id, ...placeMenu(p.x, p.y) };
  }

  function openDateMenu() {
    // 期間の絞り込みは履歴だけ
    if (!dateBtn || tab.kind !== "history") return;
    const r = dateBtn.getBoundingClientRect();
    menu = { kind: "date", x: r.left, y: r.bottom + 6 };
  }

  async function openLink(item: Item) {
    menu = null;
    try {
      await api.openUrl(item.preview.trim());
      api.hide();
    } catch (err) {
      showToast(String(err));
    }
  }

  function openKindMenu() {
    if (!kindBtn) return;
    const r = kindBtn.getBoundingClientRect();
    menu = { kind: "kind", x: r.left, y: r.bottom + 6 };
  }

  function setKind(k: KindFilter | null) {
    kindFilter = k;
    reload(false);
  }

  /** 続けて貼る: 選んだ順に、⌘V / Ctrl+V のたびに次がクリップボードに入る */
  async function startQueue(ids = targetIds()) {
    menu = null;
    try {
      const n = await api.startQueue(ids);
      if (n) showToast(`${n}件を順番に貼れます。${MOD}V のたびに次へ進みます`);
    } catch (err) {
      showToast(String(err));
    }
  }

  /** Space: 大きく表示（もう一度 Space / Esc で閉じる） */
  async function togglePreview() {
    preview = !preview;
    if (preview) await loadPreview();
  }

  async function loadPreview() {
    const it = items[focusIdx];
    previewText = it && it.kind === "text" ? ((await api.itemText(it.id)) ?? "") : null;
  }

  function setDateRange(r: DateRange) {
    dateRange = r;
    reload(false);
  }

  function openAppMenu() {
    const r = appBtn.getBoundingClientRect();
    menu = { kind: "app", x: r.left, y: r.bottom + 6 };
  }

  function openTagMenu() {
    const item = items[focusIdx];
    if (!item) return;
    const p = focusedCardPos();
    menu = { kind: "tag", itemId: item.id, ...placeMenu(p.x, p.y, 220) };
  }

  const menuItem = $derived(menu?.itemId != null ? items.find((i) => i.id === menu!.itemId) ?? null : null);

  function tagEntries(item: Item): MenuEntry[] {
    const ids = selectedSet.has(item.id) && selected.length > 1 ? selected : [item.id];
    const targets = items.filter((i) => ids.includes(i.id));
    return [
      ...tags.map((t): MenuEntry => ({
        label: t.name,
        dot: t.color,
        checked: targets.length > 0 && targets.every((i) => i.tags.includes(t.id)),
        keepOpen: true,
        run: () => toggleTagOn(item, t.id),
      })),
      ...(tags.length ? [{ kind: "sep" } as MenuEntry] : []),
      { label: "新しいタグを作成して付ける…", icon: "plus", run: () => startNewTag(item) },
      ...(tags.length
        ? [{
            label: "タグの名前・色を編集…",
            icon: "edit",
            keepOpen: true,
            run: () => {
              const m = menu;
              if (m) menu = { kind: "tagedit", x: m.x, y: m.y };
            },
          } as MenuEntry]
        : []),
    ];
  }

  const menuEntries = $derived.by((): MenuEntry[] => {
    if (!menu) return [];
    if (menu.kind === "app") {
      return [
        { label: "すべてのアプリ", icon: "apps", checked: !sourceId, run: () => setSource(null) },
        ...apps.map((a): MenuEntry => ({
          label: a.source_name,
          img: a.source_icon ? fileSrc(a.source_icon) : undefined,
          icon: a.source_id === "clipo.screenshot" ? "camera" : a.source_id === "clipo.ocr" ? "scan" : "apps",
          checked: sourceId === a.source_id,
          count: a.count,
          run: () => setSource(a.source_id),
        })),
      ];
    }
    if (menu.kind === "kind") {
      return [
        { label: "すべての種類", icon: "filter", checked: !kindFilter, run: () => setKind(null) },
        ...(Object.keys(KIND_LABEL) as KindFilter[]).map((k): MenuEntry => ({
          label: KIND_LABEL[k],
          icon: KIND_ICON[k],
          checked: kindFilter === k,
          hint: `/${KIND_LABEL[k]}`,
          run: () => setKind(k),
        })),
      ];
    }
    if (menu.kind === "transform") {
      return TRANSFORMS.map(([key, label]): MenuEntry => ({ label, icon: "text", run: () => paste(targetIds(), false, false, key) }));
    }
    if (menu.kind === "date") {
      return (Object.keys(DATE_RANGE_LABEL) as DateRange[]).map((r): MenuEntry => ({
        label: DATE_RANGE_LABEL[r],
        icon: "calendar",
        checked: dateRange === r,
        run: () => setDateRange(r),
      }));
    }
    if (menu.kind === "moretags") {
      return [
        ...hiddenTags.map((t): MenuEntry => ({
          label: t.name,
          dot: t.color,
          checked: tab.kind === "saved" && tab.tag === t.id,
          run: () => setTab({ kind: "saved", tag: t.id }),
        })),
        { kind: "sep" },
        {
          label: "タグの名前・色を編集…",
          icon: "edit",
          keepOpen: true,
          run: () => {
            const m = menu;
            if (m) menu = { kind: "tagedit", x: m.x, y: m.y };
          },
        },
      ];
    }
    if (menu.kind === "tagedit") {
      // 編集するタグを選ぶ
      return tags.map((t): MenuEntry => ({
        label: t.name,
        dot: t.color,
        keepOpen: true,
        run: () => {
          const m = menu;
          if (m) menu = { kind: "tagpill", tagId: t.id, x: m.x, y: m.y };
        },
      }));
    }
    if (menu.kind === "tagpill") {
      const t = tags.find((x) => x.id === menu!.tagId);
      if (!t) return [];
      return [
        { label: "名前を変更…", icon: "edit", run: () => startRename(t) },
        { kind: "swatches", label: "色", colors: TAG_COLORS, value: t.color, onpick: (c) => recolorTag(t, c) },
        { kind: "sep" },
        { label: "このタグを削除（カードは残ります）", icon: "trash", danger: true, run: () => deleteTag(t) },
      ];
    }
    const item = menuItem;
    if (!item) return [];
    if (menu.kind === "tag") return tagEntries(item);
    const n = selected.length > 1 ? `（${selected.length}件）` : "";
    const list: MenuEntry[] = [
      { label: `貼り付け${n}`, icon: "paste", hint: K.enter, run: () => paste() },
      { label: "コピーのみ", icon: "copy", hint: `${MOD}C`, run: () => { paste(targetIds(), true); showToast("クリップボードにコピーしました"); } },
      { label: item.saved ? `保存から外す${n}` : `保存${n}`, icon: item.saved ? "star-fill" : "star", hint: `${MOD}D`, run: () => toggleSaved() },
      { label: item.saved ? "タイトル・内容を編集…" : "名前を付けて保存…", icon: "edit", hint: `${MOD}E`, run: () => openEditor(item) },
    ];
    if (selected.length > 1) {
      list.splice(1, 0, { label: `順番に貼る（${MOD}V ごとに1件）`, icon: "queue", run: () => startQueue() });
    }
    if (item.kind === "file") {
      list.push({ label: "パスをテキストで貼り付け", icon: "text", run: () => paste(targetIds(), false, true) });
    } else if (item.kind === "text" && !item.translated && isLink(item.preview)) {
      list.push({ label: "ブラウザで開く", icon: "external", hint: `${MOD}T`, run: () => openLink(item) });
    } else if (item.kind === "text") {
      list.push({
        label: "形を変えて貼り付け…",
        icon: "edit",
        keepOpen: true,
        run: () => {
          const m = menu;
          if (m) menu = { kind: "transform", itemId: item.id, x: m.x + 16, y: m.y };
        },
      });
      list.push(
        item.translated
          ? { label: "元の文に戻す", icon: "globe", hint: `${MOD}T`, run: () => revert(item) }
          : { label: "翻訳", icon: "globe", hint: `${MOD}T`, run: () => runAction(item) },
      );
    } else {
      list.push({ label: "文字起こし", icon: "scan", hint: `${MOD}T`, run: () => runAction(item) });
    }
    const tagNames = tags.filter((t) => item.tags.includes(t.id)).map((t) => t.name);
    list.push({
      label: tagNames.length ? `タグ：${tagNames.join("、")}` : "タグ…",
      icon: "tag",
      hint: `${MOD}G`,
      keepOpen: true,
      run: () => {
        const m = menu;
        if (m) menu = { kind: "tag", itemId: item.id, x: m.x + 16, y: m.y };
      },
    });
    list.push({ kind: "sep" }, { label: `削除${n}`, icon: "trash", hint: `${MOD}${K.del}`, danger: true, run: () => remove() });
    return list;
  });

  async function startRename(t: Tag) {
    menu = null;
    renaming = { id: t.id, name: t.name };
    await tick();
    renameInput?.focus();
    renameInput?.select();
  }

  async function commitRename() {
    const r = renaming;
    renaming = null;
    const t = r && tags.find((x) => x.id === r.id);
    if (!r || !t || !r.name.trim() || r.name.trim() === t.name) return;
    try {
      await api.updateTag(t.id, r.name, t.color);
      showToast(`タグ名を「${r.name.trim()}」に変更しました`);
    } catch (err) {
      showToast(String(err));
    }
    await reload();
  }

  async function recolorTag(t: Tag, color: string) {
    await api.updateTag(t.id, t.name, color);
    await reload();
  }

  /** 選択中のタグタブのメニュー（キーボード用） */
  function openTagPillMenu() {
    if (tab.kind !== "saved" || tab.tag == null) {
      if (!tags.length) {
        showToast("タグがまだありません（＋ または " + MOD + "G から作成）", undefined, 2500);
        return;
      }
      const p = focusedCardPos();
      menu = { kind: "tagedit", ...placeMenu(p.x, p.y, 200) };
      return;
    }
    const el = document.querySelector<HTMLElement>(`[data-tag="${tab.tag}"]`);
    const r = el?.getBoundingClientRect();
    menu = { kind: "tagpill", tagId: tab.tag, ...placeMenu(r?.left ?? 40, (r?.bottom ?? 40) + 6, 260) };
  }

  async function deleteTag(t: Tag) {
    await api.deleteTag(t.id);
    if (tab.kind === "saved" && tab.tag === t.id) tab = { kind: "saved", tag: null };
    reload();
  }

  function openMenu(e: MouseEvent, idx: number) {
    e.preventDefault();
    openCardMenu(idx, { x: e.clientX, y: e.clientY });
  }

  /** 保存の編集画面。item が履歴なら、保存すると保存に移る。null なら新規 */
  async function openEditor(item: Item | null) {
    menu = null;
    if (!item) {
      editor = { id: null, title: "", keyword: "", text: "" };
      return;
    }
    const text = item.kind === "text" ? ((await api.itemText(item.id)) ?? "") : null;
    editor = { id: item.id, title: item.title ?? "", keyword: item.keyword ?? "", text };
  }

  /** 差し込み記号を本文のカーソル位置に入れる */
  async function insertPlaceholder(ph: string) {
    const el = editorText;
    if (!editor || editor.text == null || !el) return;
    const a = el.selectionStart ?? editor.text.length;
    const b = el.selectionEnd ?? a;
    editor.text = editor.text.slice(0, a) + ph + editor.text.slice(b);
    await tick();
    el.focus();
    el.setSelectionRange(a + ph.length, a + ph.length);
  }

  async function saveEditor() {
    if (!editor) return;
    const ed = editor;
    try {
      const id = await api.saveItem(ed.id, ed.title, ed.keyword, ed.text);
      // タグで絞り込み中に新しく作ったら、そのタグを付ける
      if (ed.id == null && tab.kind === "saved" && tab.tag != null) await api.setTag([id], tab.tag, true);
      editor = null;
      if (tab.kind !== "saved") {
        showToast("保存しました", { label: "保存を開く", run: () => setTab({ kind: "saved", tag: null }) });
      }
      await reload();
    } catch (err) {
      showToast(String(err));
    }
  }

  function sameTab(a: Tab, b: Tab): boolean {
    return a.kind === b.kind && (a.kind !== "saved" || (b.kind === "saved" && a.tag === b.tag));
  }

  /** Tab は履歴と保存の行き来だけ（タグは検索欄の #名前 かクリックで選ぶ） */
  function cycleTab() {
    setTab(tab.kind === "history" ? { kind: "saved", tag: null } : { kind: "history" });
  }

  /** タグのボタンを入るだけ並べる（残りは「その他 ▾」） */
  let tagZoneW = $state(0);
  let measureEl = $state<HTMLDivElement | null>(null);
  let moreBtn = $state<HTMLButtonElement | null>(null);
  let tagWidths = $state<number[]>([]);
  $effect(() => {
    tags;
    tick().then(() => {
      tagWidths = measureEl ? [...measureEl.children].map((c) => (c as HTMLElement).offsetWidth) : [];
    });
  });
  const fitCount = $derived.by(() => {
    const GAP = 6, PLUS = (addingTag ? 116 : 28) + GAP, MORE = 96 + GAP;
    const n = tags.length;
    let used = PLUS;
    for (let i = 0; i < n; i++) {
      const w = (tagWidths[i] ?? 80) + GAP;
      const rest = i < n - 1 ? MORE : 0; // まだ後ろがあるなら「その他」の分を空けておく
      if (used + w + rest > tagZoneW) return i;
      used += w;
    }
    return n;
  });
  const shownTags = $derived(tags.slice(0, fitCount));
  const hiddenTags = $derived(tags.slice(fitCount));

  function openMoreTags() {
    const r = moreBtn?.getBoundingClientRect();
    menu = { kind: "moretags", ...placeMenu(r?.left ?? 40, (r?.bottom ?? 40) + 6, 260) };
  }

  /** 検索欄の #名前 に当てはまるタグ（タグのボタンを光らせる） */
  const tagWords = $derived(
    query.split(/\s+/).filter((w) => /^[#＃]./.test(w)).map((w) => w.slice(1).toLowerCase()),
  );
  const tagHit = (t: Tag) => tagWords.some((w) => t.name.toLowerCase().startsWith(w));
  /**
   * #名前 の候補一覧。tagPrefix は自分で打った部分（候補はこれで決まる）。
   * ↑↓ で選ぶと検索欄の #… がそのタグ名に置き換わり、すぐ絞り込まれる。Enter は普段どおり貼り付け
   */
  let tagPrefix = $state<string | null>(null);
  let tagOpen = $state(false);
  let tagIdx = $state(-1);
  const tagHits = $derived(
    tagPrefix == null ? [] : tags.filter((t) => t.name.toLowerCase().startsWith(tagPrefix!.toLowerCase())),
  );
  const TAG_TAIL = /(?:^|\s)[#＃](\S*)$/;

  /** 検索欄の入力に合わせて候補を開く/閉じる */
  function syncTagPrefix() {
    tagPrefix = query.match(TAG_TAIL)?.[1] ?? null;
    tagOpen = tagPrefix != null;
    tagIdx = -1;
  }

  function closeTags() {
    tagOpen = false;
  }

  /** 候補のタグで絞り込む（検索欄の末尾の #… を置き換える） */
  function applyTag(t: Tag) {
    query = query.replace(/[#＃]\S*$/, `#${t.name}`);
    reload(false);
  }

  function moveTag(d: 1 | -1) {
    const n = tagHits.length;
    tagIdx = tagIdx < 0 ? (d > 0 ? 0 : n - 1) : (tagIdx + d + n) % n;
    applyTag(tagHits[tagIdx]);
    tick().then(() => document.querySelector(".taghits .cur")?.scrollIntoView({ block: "nearest" }));
  }

  let composing = false;
  let composeEndedAt = 0;
  function onCompositionStart() {
    composing = true;
  }
  function onCompositionEnd() {
    composing = false;
    composeEndedAt = performance.now();
  }
  function isImeKey(e: KeyboardEvent): boolean {
    return e.isComposing || composing || e.keyCode === 229 || performance.now() - composeEndedAt < 80;
  }

  function clearMultiSelect() {
    const it = items[focusIdx];
    selected = it ? [it.id] : [];
    anchorIdx = focusIdx;
  }

  function onKeydown(e: KeyboardEvent) {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    // 日本語入力の変換中・確定直後のキーは無視する
    // （WebKit は確定の Enter を isComposing=false / keyCode 229 で送ってくる）
    if (isImeKey(e)) return;

    if (editor) {
      if (e.key === "Escape") editor = null;
      else if (mod && e.key === "Enter") saveEditor();
      return;
    }
    if (addingTag || renaming) return;
    if (help) {
      if (e.key === "Escape" || (mod && e.key === "/") || e.key === "?") { e.preventDefault(); help = false; }
      return;
    }
    if (menu && menuRef?.handleKey(e)) return;
    if (menu && (e.key === "ArrowRight" || e.key.length === 1)) return;

    // 大きく表示中: Space / Esc で閉じる。←→ で隣へ（中身も入れ替わる）
    if (preview) {
      if (e.key === " " || e.key === "Escape") {
        e.preventDefault();
        preview = false;
        return;
      }
      if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        move(e.key === "ArrowRight" ? 1 : -1, false);
        loadPreview();
        return;
      }
    }
    // 検索欄が空のときの Space は「大きく表示」
    if (e.key === " " && !query && !mod && !e.shiftKey) {
      e.preventDefault();
      if (items[focusIdx]) togglePreview();
      return;
    }

    // #名前 の候補: ↑↓ で選ぶ（選んだ時点で絞り込み）。←→ でカードへ移ると閉じる。Enter は普段どおり貼り付け
    if (!mod && !e.altKey && tagPrefix != null && tagHits.length) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        // 閉じていても、末尾が #… のままなら ↓ で開き直す
        if (tagOpen || (e.key === "ArrowDown" && TAG_TAIL.test(query))) {
          e.preventDefault();
          tagOpen = true;
          moveTag(e.key === "ArrowDown" ? 1 : -1);
          return;
        }
      }
    }
    if (tagOpen) {
      if (e.key === "Escape") {
        e.preventDefault();
        closeTags();
        return;
      }
      if (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "Enter" || e.key === "Tab") closeTags();
    }

    // ⌥←→（Windows は Alt+←→）で検索欄の文字カーソルを動かす（←→ 単体はカード移動）
    if (e.altKey && !mod && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      moveCaret(e.key === "ArrowRight" ? 1 : -1, e.shiftKey);
      return;
    }

    // メニューキー / Shift+F10 で右クリックメニュー
    if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
      e.preventDefault();
      openCardMenu();
      return;
    }
    // Delete キー単体でも削除（検索文字が空のとき）
    if (e.key === "Delete" && !query) {
      e.preventDefault();
      remove();
      return;
    }

    switch (e.key) {
      case "Escape":
        e.preventDefault();
        if (menu) menu = null;
        else if (selected.length > 1) clearMultiSelect();
        else if (query) { query = ""; syncTagPrefix(); reload(false); }
        else api.hide();
        return;
      case "ArrowRight":
      case "ArrowLeft": {
        e.preventDefault();
        const dir = e.key === "ArrowRight" ? 1 : -1;
        // ⌘←/⌘→（Windows は Ctrl）で履歴の最初/最後へ
        if (mod) moveTo(dir > 0 ? items.length - 1 : 0, e.shiftKey);
        else move(dir, e.shiftKey);
        return;
      }
      case "ArrowUp":
      case "ArrowDown":
        if (paged) {
          e.preventDefault();
          moveTo(verticalTarget(focusIdx, e.key === "ArrowDown" ? 1 : -1), e.shiftKey);
        }
        return;
      case "Home":
      case "End": {
        e.preventDefault();
        const home = e.key === "Home";
        if (paged) {
          // 多段: いまの段の先頭/末尾へ
          moveTo(rowEdge(focusIdx, !home), e.shiftKey);
        } else {
          // 1段: 見えているカードの先頭/末尾へ
          const [a, b] = visibleRange();
          moveTo(home ? a : b, e.shiftKey);
        }
        return;
      }
      case "PageDown":
      case "PageUp": {
        e.preventDefault();
        const down = e.key === "PageDown";
        // 多段: 段数ぶん上下に（列はそのまま） / 1段: 見えている枚数ぶん左右に
        let target: number;
        if (paged) {
          // 前/次のページの同じ位置へ
          target = cellToIndex(focusIdx + lead + (down ? layout.perPage : -layout.perPage));
        } else {
          const n = pageSize();
          target = Math.max(0, Math.min(items.length - 1, focusIdx + (down ? n : -n)));
        }
        moveTo(target, e.shiftKey, down ? "end" : "start");
        return;
      }
      case "Enter":
        e.preventDefault();
        paste(targetIds(), e.shiftKey);
        return;
      case "Tab":
        e.preventDefault();
        cycleTab();
        return;
    }

    if (mod) {
      const k = e.key.toLowerCase();
      if (/^[1-9]$/.test(k)) {
        e.preventDefault();
        const it = items[Number(k) - 1];
        if (it) paste([it.id]);
      } else if (k === "c" && !(document.activeElement === searchEl && searchEl.selectionStart !== searchEl.selectionEnd)) {
        e.preventDefault();
        paste(targetIds(), true);
        showToast("クリップボードにコピーしました");
      } else if (k === "a" && !query) {
        e.preventDefault();
        selected = items.map((i) => i.id);
      } else if (k === "d") {
        e.preventDefault();
        toggleSaved();
      } else if (k === "t") {
        e.preventDefault();
        const it = items[focusIdx];
        if (it?.translated) revert(it);
        else if (it) runAction(it);
      } else if (k === "k" && e.shiftKey) {
        e.preventDefault();
        openTagPillMenu();
      } else if (k === "k") {
        e.preventDefault();
        openCardMenu();
      } else if (k === "[" || k === "]") {
        e.preventDefault();
        jumpDay(k === "]" ? 1 : -1);
      } else if (k === "i") {
        e.preventDefault();
        openKindMenu();
      } else if (k === "y") {
        e.preventDefault();
        openDateMenu();
      } else if (k === "o") {
        e.preventDefault();
        openAppMenu();
      } else if (k === "g") {
        e.preventDefault();
        openTagMenu();
      } else if (k === "e") {
        e.preventDefault();
        const it = items[focusIdx];
        if (it) openEditor(it);
      } else if (k === "/") {
        e.preventDefault();
        help = true;
      } else if ((k === "backspace" && !query) || k === "delete") {
        e.preventDefault();
        remove();
      } else if (k === "n") {
        e.preventDefault();
        openEditor(null);
      } else if (k === ",") {
        e.preventDefault();
        api.openSettings();
      } else if (k === "f") {
        e.preventDefault();
        searchEl.focus();
      }
      return;
    }

    // 念のため：検索欄から外れていたら戻す（日本語入力を正しく始めるため）
    if (e.key.length === 1 && document.activeElement !== searchEl) {
      searchEl.focus();
    }
  }

  function moveCaret(dir: 1 | -1, extend: boolean) {
    const el = searchEl;
    if (!el) return;
    el.focus();
    const len = el.value.length;
    const start = el.selectionStart ?? len;
    const end = el.selectionEnd ?? len;
    // 選択の「動く側」を覚えておく
    const dirAttr = el.selectionDirection;
    const head = dirAttr === "backward" ? start : end;
    const anchor = dirAttr === "backward" ? end : start;
    if (!extend && start !== end) {
      const p = dir > 0 ? end : start;
      el.setSelectionRange(p, p);
      return;
    }
    const next = Math.max(0, Math.min(len, head + dir));
    if (extend) {
      el.setSelectionRange(Math.min(anchor, next), Math.max(anchor, next), next < anchor ? "backward" : "forward");
    } else {
      el.setSelectionRange(next, next);
    }
  }

  /** 日本語入力が 1 文字目から正しく効くよう、検索欄に常にカーソルを置いておく */
  function keepSearchFocus() {
    if (!editor && !addingTag && !renaming) searchEl?.focus({ preventScroll: true });
  }

  function onPanelMouseDown(e: MouseEvent) {
    const t = e.target as HTMLElement;
    if (!t.closest("input, textarea, select")) e.preventDefault();
  }

  let searchTimer: ReturnType<typeof setTimeout>;
  function onSearch() {
    syncTagPrefix();
    // タグは保存の中の分類なので、#名前 を打ったら保存タブで探す
    if (tab.kind === "history" && /(?:^|\s)[#＃]/.test(query)) tab = { kind: "saved", tag: null };
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => reload(false), 120);
  }

  function onWheel(e: WheelEvent) {
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
      listEl.scrollLeft += e.deltaY;
      e.preventDefault();
    }
  }

  // 編集画面やタグ入力を閉じたら検索欄にカーソルを戻す
  $effect(() => {
    if (!editor && !addingTag && !renaming && !help) tick().then(keepSearchFocus);
  });

  async function applySettings() {
    const s = await api.getSettings();
    screenshotEnabled = s.screenshot_enabled;
    maxRows = s.panel_rows;
    groupByDate = s.group_by_date;
    opacity = s.panel_opacity;
    cardSize = s.card_size;
  }

  onMount(() => {
    reload(false);
    applySettings();
    api.getPaused().then((p) => (paused = p));
    const timer = setInterval(() => (now = Date.now()), 5000);
    const unsubs = [
      listen("clips-changed", () => reload()),
      listen<boolean>("paused-changed", (e) => (paused = e.payload)),
      listen("settings-changed", applySettings),
      listen("panel-shown", async () => {
        now = Date.now();
        menu = null;
        help = false;
        addingTag = false;
        editor = null;
        preview = false;
        query = "";
        syncTagPrefix();
        await reload(false);
        listEl?.scrollTo({ left: 0 });
        await tick();
        keepSearchFocus();
      }),
      listen("need-accessibility", () =>
        showToast(
          "自動貼り付けには「アクセシビリティ」の許可が必要です。いまはコピーだけしました。",
          { label: "許可する", run: () => api.openAccessibility() },
          8000,
        ),
      ),
      listen<string>("app-error", (e) => showToast(e.payload, undefined, 6000)),
    ];
    return () => {
      clearInterval(timer);
      unsubs.forEach((p) => p.then((f) => f()));
    };
  });
</script>

<svelte:window bind:innerWidth={panelWidth} onkeydown={onKeydown} oncompositionstart={onCompositionStart} oncompositionend={onCompositionEnd} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div
  class="panel"
  style:--rows={rows}
  style:--card-w="{CARD_W}px"
  style:--lines={CARD_SIZES[cardSize].lines}
  style:--glass="rgba(22, 23, 28, {opacity / 100})"
  onclick={() => { menu = null; closeTags(); }}
  onmousedown={onPanelMouseDown}
>
  <div class="bar">
    <span class="logo">Clipo</span>
    {#if paused}
      <button class="pausedpill" title="クリックで記録を再開" onclick={() => api.setPaused(false)}>
        <Icon name="pause" size={11} />記録停止中
      </button>
    {/if}

    <div class="appfilter">
      <button
        class="appbtn"
        bind:this={appBtn}
        class:active={sourceId}
        title="コピー元アプリで絞り込み（{MOD}O）"
        onclick={(e) => { e.stopPropagation(); if (menu?.kind === "app") menu = null; else openAppMenu(); }}
      >
        {#if currentApp?.source_icon}
          <img src={fileSrc(currentApp.source_icon)} alt="" />
        {:else if currentApp}
          <span class="mini">{currentApp.source_name.slice(0, 2)}</span>
        {:else}
          <Icon name="apps" size={14} />
        {/if}
        <Icon name="chevron" size={12} />
      </button>
    </div>

    <!-- 保存タブでも場所は残す（タブ切り替えで位置がずれないように） -->
    <button
      class="appbtn datebtn"
      bind:this={dateBtn}
      class:active={dateRange !== "all" && tab.kind === "history"}
      disabled={tab.kind !== "history"}
      title={tab.kind === "history" ? `期間で絞り込み（${MOD}Y）` : "期間の絞り込みは履歴だけです"}
      onclick={(e) => { e.stopPropagation(); if (menu?.kind === "date") menu = null; else openDateMenu(); }}
    >
      <Icon name="calendar" size={14} />
      {#if dateRange !== "all"}<span>{DATE_RANGE_LABEL[dateRange]}</span>{/if}
      <Icon name="chevron" size={12} />
    </button>

    <button
      class="appbtn datebtn"
      bind:this={kindBtn}
      class:active={kindFilter}
      title="種類で絞り込み（{MOD}I・検索欄に /画像 なども可）"
      onclick={(e) => { e.stopPropagation(); if (menu?.kind === "kind") menu = null; else openKindMenu(); }}
    >
      <Icon name={kindFilter ? KIND_ICON[kindFilter] : "filter"} size={14} />
      {#if kindFilter}<span>{KIND_LABEL[kindFilter]}</span>{/if}
      <Icon name="chevron" size={12} />
    </button>

    <div class="tabs" role="tablist">
      <div class="seg">
        <button class:on={tab.kind === "history"} onclick={() => setTab({ kind: "history" })}>
          <Icon name="text" size={12} />履歴
        </button>
        <button class:on={tab.kind === "saved"} onclick={() => setTab({ kind: "saved", tag: null })}>
          <span class="star"><Icon name="star-fill" size={12} /></span>保存
        </button>
      </div>
      {#if tab.kind === "saved"}
      <span class="tagsep"></span>
      <button class="pill" class:on={tab.tag == null} onclick={() => setTab({ kind: "saved", tag: null })}>すべて</button>
      {/if}
      <!-- タグ: 入るだけ並べ、入りきらない分は「その他 ▾」へ -->
      <div class="tagzone" bind:clientWidth={tagZoneW}>
        {#if tab.kind === "saved"}
        {#each shownTags as t (t.id)}
          {#if renaming?.id === t.id}
            <input
              class="taginput"
              bind:this={renameInput}
              bind:value={renaming.name}
              onkeydown={(e) => {
                if (isImeKey(e)) return;
                if (e.key === "Enter" || e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  if (e.key === "Enter") commitRename();
                  else renaming = null;
                }
              }}
              onblur={() => { if (renaming) renaming = null; }}
            />
          {:else}
          <button
            data-tag={t.id}
            class="pill"
            class:on={tab.kind === "saved" && tab.tag === t.id}
            class:hit={tagHit(t)}
            onclick={() => setTab({ kind: "saved", tag: t.id })}
            oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); menu = { kind: "tagpill", tagId: t.id, ...placeMenu(e.clientX, e.clientY, 100) }; }}
            ondblclick={() => startRename(t)}
            title="右クリック: 名前・色の変更、削除（{MOD}{K.shift}K） / ダブルクリック: 名前を変更"
          ><i style:background={t.color}></i>{t.name}</button>
          {/if}
        {/each}
        {#if hiddenTags.length}
          {@const cur = hiddenTags.find((t) => tab.kind === "saved" && tab.tag === t.id)}
          <button
            class="pill more"
            class:on={!!cur}
            class:hit={hiddenTags.some(tagHit)}
            bind:this={moreBtn}
            title="ほかのタグ"
            onclick={(e) => { e.stopPropagation(); if (menu?.kind === "moretags") menu = null; else openMoreTags(); }}
          >
            {#if cur}<i style:background={cur.color}></i>{cur.name}{:else}その他 {hiddenTags.length}{/if}
            <Icon name="chevron" size={12} />
          </button>
        {/if}
        {/if}
        {#if addingTag}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="taginput"
            placeholder={newTagTargets.length ? "付けるタグ名" : "新しいタグ名"}
            bind:this={tagInput}
            bind:value={newTagName}
            autofocus
            onkeydown={(e) => {
              if (isImeKey(e)) return;
              if (e.key === "Enter" || e.key === "Escape") {
                // パネル側の「貼り付け/閉じる」に伝わらないようにする
                e.preventDefault();
                e.stopPropagation();
                if (e.key === "Enter") commitNewTag();
                else cancelNewTag();
                tick().then(keepSearchFocus);
              }
            }}
            onblur={() => { if (addingTag) cancelNewTag(); }}
          />
        {:else if tab.kind === "saved"}
          <button class="plus" title="タグを作成" onclick={() => startNewTag()}><Icon name="plus" size={14} /></button>
        {/if}
      </div>
      <!-- 幅を測るためだけの見えない並び -->
      <div class="tagmeasure" aria-hidden="true" bind:this={measureEl}>
        {#each tags as t (t.id)}<span class="pill"><i></i>{t.name}</span>{/each}
      </div>
    </div>

    <div class="tools">
      <label class="search">
        <Icon name="search" size={14} />
        <input bind:this={searchEl} bind:value={query} oninput={onSearch} placeholder="検索（#でタグ）" spellcheck="false" />
        {#if tagOpen}
          <div class="taghits">
            {#each tagHits as t, n (t.id)}
              <button class:cur={n === tagIdx} onclick={(e) => { e.stopPropagation(); tagIdx = n; applyTag(t); closeTags(); keepSearchFocus(); }}>
                <i style:background={t.color}></i>{t.name}
              </button>
            {:else}
              <span>{tags.length ? "一致するタグはありません" : "タグがまだありません"}</span>
            {/each}
            {#if tagHits.length}<small>↑↓ でタグ・←→ でカード・{K.enter} で貼り付け</small>{/if}
          </div>
        {/if}
      </label>
      <button class="tool" title="範囲を選んで文字起こし" onclick={() => api.ocrCapture()}><Icon name="scan" size={16} /></button>
      {#if screenshotEnabled}
        <button class="tool" title="範囲スクショ" onclick={() => api.screenshot()}><Icon name="camera" size={16} /></button>
      {/if}
      <button class="tool" title="設定 ({MOD},)" onclick={() => api.openSettings()}><Icon name="settings" size={16} /></button>
    </div>
  </div>

  {#snippet newCard()}
    <button class="newcard" onclick={() => openEditor(null)}>
      <Icon name="plus" size={22} />
      <span>テキストを作って保存</span>
      <small>{MOD}N</small>
    </button>
  {/snippet}

  {#snippet card(i: number)}
    {@const item = items[i]}
    <Card
      {item}
      {tags}
      index={i}
      {now}
      selected={selectedSet.has(item.id)}
      focused={i === focusIdx}
      busy={busy[item.id]}
      order={selected.length > 1 && selectedSet.has(item.id) ? selected.indexOf(item.id) + 1 : null}
      dimmed={selected.length > 1 && !selectedSet.has(item.id)}
      onclick={(e) => clickCard(e, i)}
      ondblclick={() => paste([item.id])}
      oncontextmenu={(e) => openMenu(e, i)}
      onaction={() => runAction(item)}
      onrevert={() => revert(item)}
    />
  {/snippet}

  <div class="list" class:paged bind:this={listEl} onwheel={onWheel} role="listbox" aria-multiselectable="true">
    {#if !items.length && !lead}
      <div class="emptylist">
        {#if query || kindFilter || sourceId || dateRange !== "all"}
          <b>当てはまるものはありません</b>
          <span>検索や絞り込みを変えてみてください{query ? `（${MOD}F で検索欄、Esc で検索をクリア）` : ""}</span>
        {:else}
          <b>ここにコピーした履歴が並びます</b>
          <span>何かをコピーするとカードになります。よく使うものは {MOD}D で「保存」へ</span>
        {/if}
      </div>
    {:else if paged}
      {#each pages as page, p (p)}
        <div
          class="page"
          data-page={p}
          style:grid-template-columns="repeat({layout.cols}, minmax(0, 1fr))"
          style:grid-template-rows="repeat({layout.rows}, {CARD_H}px)"
        >
          {#each page as cell (cell === -1 ? "new" : items[cell].id)}
            {#if cell === -1}
              {@render newCard()}
            {:else}
              <div class="cell">
                {#if isDayStart(cell)}<span class="day">{dayLabel(items[cell].used_at, now)}</span>{/if}
                {@render card(cell)}
              </div>
            {/if}
          {/each}
        </div>
      {/each}
    {:else}
      {#if lead}{@render newCard()}{/if}
      {#each items as item, i (item.id)}
        {#if isDayStart(i)}
          <div class="daysep"><span>{dayLabel(item.used_at, now)}</span></div>
        {/if}
        {@render card(i)}
      {/each}
    {/if}
  </div>

  <div class="hints">
    {#if selected.length > 1}
      <span class="multi">{selected.length}件を選択中</span>
      <button class="pastebtn" onclick={() => paste()}>まとめて貼り付け {K.enter}</button>
      <button class="pastebtn ghostbtn" onclick={() => startQueue()}>順番に貼る（{MOD}V ごと）</button>
      <button class="link" onclick={clearMultiSelect}>選択解除（Esc）</button>
    {:else}
      <span><b>←→</b> 移動</span>
      <span><b>{K.shift}←→</b> 複数選択</span>
      <span><b>{K.enter}</b> 貼り付け</span>
      <span><b>{MOD}1-9</b> 直接貼り付け</span>
      <span><b>{MOD}K</b> メニュー</span>
      <span><b>{MOD}T</b> 翻訳</span>
      <span><b>Space</b> 大きく表示</span>
      <span><b>{MOD}D</b> 保存</span>
      <span><b>{MOD}G</b> タグ</span>
      <span><b>{MOD}O</b> アプリ</span>
      <span><b>{MOD}Y</b> 期間</span>
      <span><b>{MOD}I</b> 種類</span>
      <span><b>{MOD}[ ]</b> 前後の日</span>
      <span><b>Tab</b> 履歴/保存</span>
      <span><b>#</b> タグで絞り込み</span>
      <span><b>{MOD}/</b> すべてのキー</span>
    {/if}
  </div>

  {#if menu && menuEntries.length}
    {#key menu.kind}
      <Menu bind:this={menuRef} entries={menuEntries} x={menu.x} y={menu.y} onclose={() => { menu = null; keepSearchFocus(); }} />
    {/key}
  {/if}

  {#if preview && items[focusIdx]}
    {@const it = items[focusIdx]}
    <div class="overlay" onclick={() => (preview = false)}>
      <div class="preview" onclick={(e) => e.stopPropagation()}>
        <header>
          <b>{it.title || (it.kind === "image" ? "画像" : it.kind === "file" ? "ファイル" : isLink(it.preview) ? "リンク" : "テキスト")}</b>
          {#if it.kind === "image" && it.width}<small>{it.width}×{it.height}</small>{/if}
          {#if previewText != null}<small>{previewText.length}字</small>{/if}
          <span class="sp"></span>
          {#if it.kind === "text" && isLink(it.preview)}
            <button class="pastebtn" onclick={() => openLink(it)}>ブラウザで開く</button>
          {/if}
          <button class="pastebtn" onclick={() => paste([it.id])}>貼り付け {K.enter}</button>
          <small>Space / Esc で閉じる・←→ で隣へ</small>
        </header>
        <div class="pbody">
          {#if it.kind === "image"}
            <img src={fileSrc(it.image_path)} alt="" draggable="false" />
          {:else if it.kind === "file"}
            <ul class="pfiles">
              {#each it.preview.split("\n").filter(Boolean) as p, n (n)}
                <li><b>{fileNames(p)[0]}</b><small>{p}</small></li>
              {/each}
            </ul>
          {:else}
            <pre>{previewText ?? it.preview}</pre>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  {#if help}
    <div class="overlay" onclick={() => (help = false)}>
      <div class="help" onclick={(e) => e.stopPropagation()}>
        <h3>キーボード操作 <small>Esc で閉じる</small></h3>
        <div class="cols">
          <dl>
            <dt>←→</dt><dd>移動</dd>
            <dt>↑↓</dt><dd>上下の段へ（2段以上）</dd>
            <dt>Home / End</dt><dd>見えている先頭・末尾へ（多段は段の端）</dd>
            <dt>PageUp / Down</dt><dd>前 / 次のページへ</dd>
            <dt>{MOD}← / {MOD}→</dt><dd>履歴の最初 / 最後へ</dd>
            <dt>{K.shift}←→</dt><dd>範囲を複数選択</dd>
            <dt>{MOD}A</dt><dd>すべて選択</dd>
            <dt>Esc</dt><dd>選択解除 → 検索クリア → 閉じる</dd>
            <dt>{K.enter}</dt><dd>貼り付け（複数なら選んだ順に）</dd>
            <dt>{K.shift}{K.enter} / {MOD}C</dt><dd>コピーのみ</dd>
            <dt>{MOD}1〜9</dt><dd>その番号のカードを貼り付け（保存はよく使う順）</dd>
          </dl>
          <dl>
            <dt>文字を入力</dt><dd>そのまま検索（{MOD}F）</dd>
            <dt>{K.alt}← / {K.alt}→</dt><dd>検索欄の文字カーソルを移動（{K.shift}で選択）</dd>
            <dt>Tab</dt><dd>履歴 / 保存 を切り替え</dd>
            <dt>#名前</dt><dd>タグで絞り込み（# だけで候補一覧、↑↓ で選ぶとすぐ絞り込み）</dd>
            <dt>{MOD}O</dt><dd>コピー元アプリで絞り込み</dd>
            <dt>{MOD}Y</dt><dd>期間で絞り込み</dd>
            <dt>{MOD}I</dt><dd>種類で絞り込み（検索欄に /画像 /リンク なども可）</dd>
            <dt>Space</dt><dd>大きく表示（←→ で隣へ・Space / Esc で閉じる）</dd>
            <dt>{MOD}[ / {MOD}]</dt><dd>前の日 / 次の日の先頭へ</dd>
            <dt>{MOD}K</dt><dd>メニュー（↑↓ {K.enter}）</dd>
            <dt>{MOD}G</dt><dd>タグを付ける / 外す・新規作成（付けると保存に入る）</dd>
            <dt>{MOD}{K.shift}K</dt><dd>タグの名前・色の変更、削除（{MOD}G のメニューからも）</dd>
            <dt>{MOD}D</dt><dd>保存する / 保存から外す</dd>
            <dt>{MOD}T</dt><dd>翻訳 / 元に戻す・画像は文字起こし・リンクは開く</dd>
            <dt>{MOD}K</dt><dd>メニュー：形を変えて貼る、順番に貼る（複数選択時）</dd>
            <dt>{MOD}E</dt><dd>タイトル・短い名前・内容を編集して保存</dd>
            <dt>{MOD}N</dt><dd>テキストを作って保存</dd>
            <dt>{MOD}{K.del}</dt><dd>削除</dd>
            <dt>{MOD},</dt><dd>設定</dd>
          </dl>
        </div>
      </div>
    </div>
  {/if}

  {#if editor}
    <div class="overlay" onclick={() => (editor = null)}>
      <div class="editor" onclick={(e) => e.stopPropagation()}>
        <!-- svelte-ignore a11y_autofocus -->
        <div class="fields">
          <input class="grow" bind:value={editor.title} placeholder="タイトル（例: 住所、署名）" autofocus />
          <input class="kwin" bind:value={editor.keyword} placeholder="短い名前（例: sig）" spellcheck="false" />
        </div>
        {#if editor.text != null}
          <textarea bind:this={editorText} bind:value={editor.text} placeholder="本文"></textarea>
          <div class="phs">
            <span>差し込み:</span>
            {#each PLACEHOLDERS as ph (ph)}
              <button onclick={() => insertPlaceholder(ph)}>{ph}</button>
            {/each}
          </div>
        {/if}
        <div class="row">
          <span class="hint">{MOD}⏎ で保存 / Esc で閉じる</span>
          <button class="link" onclick={() => (editor = null)}>キャンセル</button>
          <button class="pastebtn" onclick={saveEditor}>保存</button>
        </div>
      </div>
    </div>
  {/if}

  {#if toast}
    <div class="toast">
      <span>{toast.text}</span>
      {#if toast.action}
        {@const a = toast.action}
        <button onclick={() => { a.run(); toast = null; }}>{a.label}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .panel {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    background: var(--glass);
    border: 1px solid var(--glass-border);
    color: var(--fg);
    overflow: hidden;
  }

  .bar {
    height: 48px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px 0 16px;
  }
  .logo { font-weight: 800; font-size: 15px; letter-spacing: 0.01em; }

  .appfilter { position: relative; }
  .appbtn {
    height: 28px; padding: 0 6px 0 7px;
    display: flex; align-items: center; gap: 3px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: var(--btn);
    color: var(--fg-dim);
  }
  .appbtn.active { border-color: var(--accent); color: var(--fg); }
  .datebtn { flex: none; gap: 4px; font-size: 12px; font-weight: 600; }
  .datebtn:disabled { opacity: 0.35; cursor: default; }
  .appbtn img { width: 16px; height: 16px; }
  .mini {
    width: 16px; height: 16px; border-radius: 50%;
    display: grid; place-items: center;
    background: #64748b; color: #fff;
    font-size: 7px; font-weight: 800; text-transform: uppercase;
  }

  .tabs {
    flex: 1; min-width: 0;
    display: flex; align-items: center; gap: 6px;
    overflow-x: auto; scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar { display: none; }
  .pill {
    flex: none;
    height: 28px; padding: 0 12px;
    display: flex; align-items: center; gap: 6px;
    border-radius: 999px;
    font-size: 12.5px; font-weight: 600;
    background: var(--btn);
    border: 1px solid var(--line);
    color: var(--fg-dim);
  }
  .pill:hover { color: var(--fg); }
  .pill.on {
    background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 85%, white), var(--accent));
    border-color: transparent;
    color: #fff;
    box-shadow: 0 2px 10px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .pill i { width: 8px; height: 8px; border-radius: 50%; }
  /* 履歴 / 保存 の切り替え */
  .seg {
    flex: none; display: flex; padding: 2px; gap: 2px;
    height: 30px; border-radius: 10px;
    background: var(--btn); border: 1px solid var(--line);
  }
  .seg button {
    display: flex; align-items: center; gap: 5px;
    padding: 0 12px; border-radius: 8px;
    font-size: 12.5px; font-weight: 700; color: var(--fg-dim);
  }
  .seg button:hover { color: var(--fg); }
  .seg button.on {
    background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 85%, white), var(--accent));
    color: #fff;
    box-shadow: 0 2px 10px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .seg .star { color: #fbbf24; display: flex; }
  .seg button.on .star { color: #fff3b0; }
  .tagzone { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; overflow: hidden; }
  .pill.more { gap: 4px; }
  .tagmeasure {
    position: absolute; left: -9999px; top: 0;
    display: flex; gap: 6px; visibility: hidden; pointer-events: none;
  }
  .tagsep { flex: none; width: 1px; height: 18px; background: var(--line-strong); margin: 0 2px; }
  .plus { width: 28px; height: 28px; flex: none; display: grid; place-items: center; border-radius: 50%; color: var(--fg-dim); }
  .plus:hover { background: var(--hover); color: var(--fg); }
  .taginput {
    width: 110px; height: 28px; padding: 0 10px; flex: none;
    border-radius: 999px; border: 1px solid var(--accent);
    background: var(--btn); color: var(--fg); font-size: 12.5px; outline: none;
  }

  .tools { display: flex; align-items: center; gap: 4px; flex: none; }
  .search {
    display: flex; align-items: center; gap: 6px;
    height: 30px; width: 190px; padding: 0 10px;
    border-radius: 9px;
    background: var(--btn);
    border: 1px solid var(--line);
    color: var(--fg-dim);
  }
  .search:focus-within { border-color: var(--accent); color: var(--fg); }
  .search input { flex: 1; min-width: 0; border: 0; outline: 0; background: transparent; color: var(--fg); font-size: 13px; }
  .search input::placeholder { color: var(--fg-dim); }
  .search { position: relative; }
  .taghits {
    position: absolute; z-index: 30; top: calc(100% + 6px); right: 0;
    min-width: 100%; max-height: 220px; overflow-y: auto; padding: 6px;
    display: flex; flex-direction: column; gap: 2px;
    border-radius: 10px; background: var(--menu); border: 1px solid var(--glass-border);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.4);
  }
  .taghits button {
    flex: none; display: flex; align-items: center; gap: 6px;
    padding: 4px 8px; border-radius: 6px; text-align: left;
    font-size: 12.5px; color: var(--fg); white-space: nowrap;
  }
  .taghits button.cur { background: var(--hover); outline: 1px solid var(--accent); }
  .taghits i { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  .taghits span, .taghits small { padding: 2px 8px; font-size: 11px; color: var(--fg-dim); white-space: nowrap; }
  .pill.hit:not(.on) { color: var(--fg); border-color: var(--accent); box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 35%, transparent); }
  .tool { width: 30px; height: 30px; display: grid; place-items: center; border-radius: 8px; color: var(--fg-dim); }
  .tool:hover { background: var(--hover); color: var(--fg); }

  .list {
    flex: 1; min-height: 0;
    display: flex;
    gap: 12px;
    padding: 12px 16px 10px;
    overflow-x: auto; overflow-y: hidden;
    scroll-padding: 0 16px;
    scrollbar-width: thin;
  }
  /* 多段: 1 ページ = 列数×段数。ページは横に並び、ページ単位で止まる */
  .list.paged { scroll-snap-type: x mandatory; scroll-padding: 0 16px; }
  .page {
    flex: 0 0 100%;
    display: grid;
    gap: 12px;
    scroll-snap-align: start;
  }
  .cell { position: relative; min-width: 0; display: flex; }
  .cell > :global(.card) { flex: 1 1 auto; }
  .page > .newcard { height: 100%; }
  .list:not(.paged) > .newcard { flex: 0 0 150px; }
  /* その日の最初のカードの左上に日付ラベル（多段・1段共通） */
  .day, .daysep span {
    position: absolute; z-index: 2; top: -9px; left: 10px;
    padding: 1px 8px; border-radius: 999px;
    background: var(--fg); color: #15161a;
    font-size: 10.5px; font-weight: 800; letter-spacing: 0.04em; white-space: nowrap;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.35);
    pointer-events: none;
  }

  /* 1段: 日付の変わり目に細い区切り線。ラベルは右隣のカードの左上に横書きで載せる */
  .daysep {
    flex: none; position: relative;
    width: 1px;
    background: var(--line);
  }
  .daysep span { left: 23px; }
  .daysep:first-child { background: transparent; margin-right: -13px; }
  .daysep:first-child span { left: 10px; }

  .newcard {
    height: 100%;
    display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px;
    border-radius: 14px;
    border: 1.5px dashed var(--line-strong);
    color: var(--fg-dim); font-size: 13px; font-weight: 600;
  }
  .newcard:hover { color: var(--fg); border-color: var(--accent); }
  .newcard small { font-family: var(--mono); opacity: 0.6; }
  .emptylist { margin: auto; display: flex; flex-direction: column; align-items: center; gap: 6px; color: var(--fg-dim); font-size: 12px; text-align: center; }
  .emptylist b { color: var(--fg); font-size: 14px; }
  .pausedpill {
    flex: none; height: 24px; padding: 0 10px; border-radius: 999px;
    display: flex; align-items: center; gap: 5px;
    background: #f59e0b; color: #1f1300; font-size: 11.5px; font-weight: 800;
  }
  .ghostbtn { background: transparent; border: 1px solid var(--accent); color: var(--fg); }
  .preview {
    width: min(900px, 94vw); height: calc(100vh - 40px);
    display: flex; flex-direction: column; overflow: hidden;
    border-radius: 14px; background: var(--menu); border: 1px solid var(--glass-border);
  }
  .preview header { flex: none; display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--line); font-size: 13px; }
  .preview header small { color: var(--fg-dim); font-size: 11px; }
  .preview .sp { flex: 1; }
  .pbody { flex: 1; min-height: 0; overflow: auto; padding: 12px 14px; }
  .pbody pre { margin: 0; white-space: pre-wrap; word-break: break-word; font: 13px/1.6 var(--mono); color: var(--fg); user-select: text; }
  .pbody img { max-width: 100%; max-height: 100%; display: block; margin: auto; object-fit: contain; }
  .pfiles { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 8px; }
  .pfiles li { display: flex; flex-direction: column; }
  .pfiles small { color: var(--fg-dim); font-size: 11px; overflow-wrap: anywhere; }

  .hints {
    height: 26px; flex: none;
    display: flex; align-items: center; gap: 14px;
    padding: 0 18px 4px;
    font-size: 11px; color: var(--fg-dim);
    white-space: nowrap; overflow: hidden;
  }
  .hints b { font-family: var(--mono); font-weight: 600; color: var(--fg); opacity: 0.8; margin-right: 2px; }
  .multi { color: var(--fg); font-weight: 700; }
  .pastebtn {
    height: 22px; padding: 0 10px; border-radius: 6px;
    background: var(--accent); color: #fff; font-size: 11.5px; font-weight: 700;
  }
  .link { color: var(--fg-dim); font-size: 11.5px; text-decoration: underline; }


  .help {
    width: min(760px, 94vw); padding: 12px 16px 14px;
    border-radius: 14px; background: var(--menu); border: 1px solid var(--glass-border);
  }
  .help h3 { margin: 0 0 8px; font-size: 13px; }
  .help h3 small { margin-left: 8px; font-weight: 400; color: var(--fg-dim); }
  .help .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 0 24px; }
  .help dl { margin: 0; display: grid; grid-template-columns: auto 1fr; gap: 3px 12px; font-size: 12px; }
  .help dt { font-family: var(--mono); font-weight: 600; color: var(--fg); white-space: nowrap; }
  .help dd { margin: 0; color: var(--fg-dim); }
  .overlay { position: fixed; inset: 0; z-index: 40; display: grid; place-items: center; background: rgba(0, 0, 0, 0.35); }
  .editor {
    width: min(560px, 92vw); padding: 12px;
    display: flex; flex-direction: column; gap: 8px;
    border-radius: 14px; background: var(--menu); border: 1px solid var(--glass-border);
  }
  .editor input, .editor textarea {
    border-radius: 8px; border: 1px solid var(--line); background: var(--btn);
    color: var(--fg); padding: 7px 10px; font: inherit; font-size: 13px; outline: none;
  }
  .editor input:focus, .editor textarea:focus { border-color: var(--accent); }
  .editor textarea { height: 110px; resize: none; }
  .editor .fields { display: flex; gap: 8px; }
  .editor .grow { flex: 1; min-width: 0; }
  .editor .kwin { width: 150px; font-family: var(--mono); }
  .editor .phs { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; font-size: 11px; color: var(--fg-dim); }
  .editor .phs button {
    padding: 2px 8px; border-radius: 6px; border: 1px solid var(--line);
    background: var(--btn); color: var(--fg); font-size: 11.5px;
  }
  .editor .phs button:hover { border-color: var(--accent); }
  .editor .row { display: flex; align-items: center; gap: 10px; }
  .editor .hint { flex: 1; font-size: 11px; color: var(--fg-dim); }
  .editor .pastebtn { height: 28px; padding: 0 16px; font-size: 12.5px; }

  .toast {
    position: fixed; left: 50%; bottom: 36px; transform: translateX(-50%); z-index: 50;
    display: flex; align-items: center; gap: 10px;
    max-width: 80vw; padding: 8px 10px 8px 14px;
    border-radius: 10px; background: #111827; color: #fff; font-size: 12.5px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.4);
  }
  .toast button { padding: 4px 10px; border-radius: 6px; background: var(--accent); color: #fff; font-weight: 700; font-size: 12px; }
</style>
