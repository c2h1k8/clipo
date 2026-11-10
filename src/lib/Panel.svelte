<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    api, fileSrc, isMac, TAG_COLORS, DATE_RANGE_LABEL, dateRangeBounds, dayKey, dayLabel,
    type DateRange, type Item, type SourceApp, type Tab, type Tag,
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
  let tab = $state<Tab>({ kind: "all" });
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
  /** 開いているメニュー（右クリック/コピー元/タグ） */
  let menu = $state<{ kind: "card" | "app" | "tag" | "tagpill" | "date"; x: number; y: number; itemId?: number; tagId?: number } | null>(null);
  let menuRef = $state<Menu | null>(null);
  let help = $state(false);
  let appBtn: HTMLButtonElement;
  let dateBtn: HTMLButtonElement;
  let tagInput = $state<HTMLInputElement | null>(null);
  let editor = $state<{ id: number | null; title: string; text: string } | null>(null);
  let screenshotEnabled = $state(true);
  /** 設定の最大段数 */
  let maxRows = $state(1);
  /** 実際の段数：1段に収まらないときだけ、最大段数まで増やす */
  let rows = $state(1);
  let panelWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1200);
  // レイアウト計算（CSS と同じ値）
  const CARD_W = 236, CARD_H = 204, GAP = 12, PAD_X = 32;
  /** バー 48 + ヒント 26 + 一覧の上下余白 22 */
  const CHROME_H = 96;
  const showDays = $derived(groupByDate && tab.kind !== "snippet");
  /** 定型文タブでは先頭に「新しい定型文」カードが入る */
  const lead = $derived(tab.kind === "snippet" ? 1 : 0);
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
  /** ページごとのセル（-1 は「新しい定型文」カード） */
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
    const [it, tg, ap] = await Promise.all([api.listItems(tab, sourceId, query, dateRangeBounds(dateRange)), api.listTags(), api.sourceApps()]);
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

  async function paste(ids = targetIds(), copyOnly = false) {
    if (!ids.length) return;
    menu = null;
    await api.paste(ids, copyOnly);
  }

  async function toggleFavorite(ids = targetIds()) {
    const list = items.filter((i) => ids.includes(i.id));
    const fav = !list.every((i) => i.favorite);
    for (const i of list) await api.setFavorite(i.id, fav);
    menu = null;
    reload();
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
    if (busy[item.id]) return;
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
      showToast(`${targets.length > 1 ? `${targets.length}件に` : ""}タグ「${t.name}」を付けました`);
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
    showToast(on ? `「${name}」を付けました` : `「${name}」を外しました`, undefined, 1800);
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
    const r = dateBtn.getBoundingClientRect();
    menu = { kind: "date", x: r.left, y: r.bottom + 6 };
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
    if (menu.kind === "date") {
      return (Object.keys(DATE_RANGE_LABEL) as DateRange[]).map((r): MenuEntry => ({
        label: DATE_RANGE_LABEL[r],
        icon: "calendar",
        checked: dateRange === r,
        run: () => setDateRange(r),
      }));
    }
    if (menu.kind === "tagpill") {
      const t = tags.find((x) => x.id === menu!.tagId);
      if (!t) return [];
      return [
        { label: `「${t.name}」で絞り込む`, dot: t.color, run: () => setTab({ kind: "tag", id: t.id }) },
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
      { label: item.favorite ? "お気に入りを外す" : "お気に入りに追加", icon: item.favorite ? "star-fill" : "star", hint: `${MOD}D`, run: () => toggleFavorite() },
    ];
    if (item.kind === "text") {
      list.push(
        item.translated
          ? { label: "元の文に戻す", icon: "globe", hint: `${MOD}T`, run: () => revert(item) }
          : { label: "翻訳", icon: "globe", hint: `${MOD}T`, run: () => runAction(item) },
        { label: item.snippet ? "定型文を編集" : "定型文として保存", icon: "edit", hint: `${MOD}E`, run: () => editSnippet(item) },
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

  async function deleteTag(t: Tag) {
    await api.deleteTag(t.id);
    if (tab.kind === "tag" && tab.id === t.id) tab = { kind: "all" };
    reload();
  }

  function openMenu(e: MouseEvent, idx: number) {
    e.preventDefault();
    openCardMenu(idx, { x: e.clientX, y: e.clientY });
  }

  async function editSnippet(item: Item | null) {
    menu = null;
    if (!item) {
      editor = { id: null, title: "", text: "" };
      return;
    }
    const text = (await api.itemText(item.id)) ?? "";
    editor = { id: item.snippet ? item.id : null, title: item.snippet ? (item.title ?? "") : "", text };
  }

  async function saveEditor() {
    if (!editor) return;
    try {
      await api.saveSnippet(editor.id, editor.title, editor.text);
      editor = null;
      setTab({ kind: "snippet" });
    } catch (err) {
      showToast(String(err));
    }
  }

  function cycleTab(dir: number) {
    const all: Tab[] = [{ kind: "all" }, { kind: "favorite" }, { kind: "snippet" }, ...tags.map((t) => ({ kind: "tag", id: t.id }) as Tab)];
    const i = all.findIndex((t) => t.kind === tab.kind && (t.kind !== "tag" || (tab.kind === "tag" && t.id === tab.id)));
    setTab(all[(i + dir + all.length) % all.length]);
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
    if (addingTag) return;
    if (help) {
      if (e.key === "Escape" || (mod && e.key === "/") || e.key === "?") { e.preventDefault(); help = false; }
      return;
    }
    if (menu && menuRef?.handleKey(e)) return;
    if (menu && (e.key === "ArrowRight" || e.key.length === 1)) return;

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
        else if (query) { query = ""; reload(false); }
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
        cycleTab(e.shiftKey ? -1 : 1);
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
        toggleFavorite();
      } else if (k === "t") {
        e.preventDefault();
        const it = items[focusIdx];
        if (it?.translated) revert(it);
        else if (it) runAction(it);
      } else if (k === "k") {
        e.preventDefault();
        openCardMenu();
      } else if (k === "[" || k === "]") {
        e.preventDefault();
        jumpDay(k === "]" ? 1 : -1);
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
        if (it?.kind === "text") editSnippet(it);
      } else if (k === "/") {
        e.preventDefault();
        help = true;
      } else if ((k === "backspace" && !query) || k === "delete") {
        e.preventDefault();
        remove();
      } else if (k === "n") {
        e.preventDefault();
        editSnippet(null);
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
    if (!editor && !addingTag) searchEl?.focus({ preventScroll: true });
  }

  function onPanelMouseDown(e: MouseEvent) {
    const t = e.target as HTMLElement;
    if (!t.closest("input, textarea, select")) e.preventDefault();
  }

  let searchTimer: ReturnType<typeof setTimeout>;
  function onSearch() {
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
    if (!editor && !addingTag && !help) tick().then(keepSearchFocus);
  });

  async function applySettings() {
    const s = await api.getSettings();
    screenshotEnabled = s.screenshot_enabled;
    maxRows = s.panel_rows;
    groupByDate = s.group_by_date;
    opacity = s.panel_opacity;
  }

  onMount(() => {
    reload(false);
    applySettings();
    const timer = setInterval(() => (now = Date.now()), 5000);
    const unsubs = [
      listen("clips-changed", () => reload()),
      listen("settings-changed", applySettings),
      listen("panel-shown", async () => {
        now = Date.now();
        menu = null;
        help = false;
        addingTag = false;
        editor = null;
        query = "";
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
  style:--glass="rgba(22, 23, 28, {opacity / 100})"
  onclick={() => (menu = null)}
  onmousedown={onPanelMouseDown}
>
  <div class="bar">
    <span class="logo">Clipo</span>

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

    <button
      class="appbtn datebtn"
      bind:this={dateBtn}
      class:active={dateRange !== "all"}
      title="期間で絞り込み（{MOD}Y）"
      onclick={(e) => { e.stopPropagation(); if (menu?.kind === "date") menu = null; else openDateMenu(); }}
    >
      <Icon name="calendar" size={14} />
      {#if dateRange !== "all"}<span>{DATE_RANGE_LABEL[dateRange]}</span>{/if}
      <Icon name="chevron" size={12} />
    </button>

    <div class="tabs" role="tablist">
      <button class="pill" class:on={tab.kind === "all"} onclick={() => setTab({ kind: "all" })}>すべて</button>
      <button class="pill" class:on={tab.kind === "favorite"} onclick={() => setTab({ kind: "favorite" })}>
        <span class="star"><Icon name="star-fill" size={11} /></span>お気に入り
      </button>
      <button class="pill" class:on={tab.kind === "snippet"} onclick={() => setTab({ kind: "snippet" })}>定型文</button>
      {#each tags as t (t.id)}
        <button
          class="pill"
          class:on={tab.kind === "tag" && tab.id === t.id}
          onclick={() => setTab({ kind: "tag", id: t.id })}
          oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); menu = { kind: "tagpill", tagId: t.id, ...placeMenu(e.clientX, e.clientY, 100) }; }}
          title="右クリックで削除"
        ><i style:background={t.color}></i>{t.name}</button>
      {/each}
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
      {:else}
        <button class="plus" title="タグを作成" onclick={() => startNewTag()}><Icon name="plus" size={14} /></button>
      {/if}
    </div>

    <div class="tools">
      <label class="search">
        <Icon name="search" size={14} />
        <input bind:this={searchEl} bind:value={query} oninput={onSearch} placeholder="検索" spellcheck="false" />
      </label>
      <button class="tool" title="範囲を選んで文字起こし" onclick={() => api.ocrCapture()}><Icon name="scan" size={16} /></button>
      {#if screenshotEnabled}
        <button class="tool" title="範囲スクショ" onclick={() => api.screenshot()}><Icon name="camera" size={16} /></button>
      {/if}
      <button class="tool" title="設定 ({MOD},)" onclick={() => api.openSettings()}><Icon name="settings" size={16} /></button>
    </div>
  </div>

  {#snippet newCard()}
    <button class="newcard" onclick={() => editSnippet(null)}>
      <Icon name="plus" size={22} />
      <span>新しい定型文</span>
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
        {query ? `「${query}」に一致する履歴はありません` : "ここにコピーした履歴が並びます"}
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
      <button class="link" onclick={clearMultiSelect}>選択解除（Esc）</button>
    {:else}
      <span><b>←→</b> 移動</span>
      <span><b>{K.shift}←→</b> 複数選択</span>
      <span><b>{K.enter}</b> 貼り付け</span>
      <span><b>{MOD}1-9</b> 直接貼り付け</span>
      <span><b>{MOD}K</b> メニュー</span>
      <span><b>{MOD}T</b> 翻訳</span>
      <span><b>{MOD}G</b> タグ</span>
      <span><b>{MOD}O</b> アプリ</span>
      <span><b>{MOD}Y</b> 期間</span>
      <span><b>{MOD}[ ]</b> 前後の日</span>
      <span><b>Tab</b> 切替</span>
      <span><b>{MOD}/</b> すべてのキー</span>
    {/if}
  </div>

  {#if menu && menuEntries.length}
    {#key menu.kind}
      <Menu bind:this={menuRef} entries={menuEntries} x={menu.x} y={menu.y} onclose={() => { menu = null; keepSearchFocus(); }} />
    {/key}
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
            <dt>{MOD}1〜9</dt><dd>その番号のカードを貼り付け</dd>
          </dl>
          <dl>
            <dt>文字を入力</dt><dd>そのまま検索（{MOD}F）</dd>
            <dt>{K.alt}← / {K.alt}→</dt><dd>検索欄の文字カーソルを移動（{K.shift}で選択）</dd>
            <dt>Tab / {K.shift}Tab</dt><dd>タブ切り替え</dd>
            <dt>{MOD}O</dt><dd>コピー元アプリで絞り込み</dd>
            <dt>{MOD}Y</dt><dd>期間で絞り込み</dd>
            <dt>{MOD}[ / {MOD}]</dt><dd>前の日 / 次の日の先頭へ</dd>
            <dt>{MOD}K</dt><dd>メニュー（↑↓ {K.enter}）</dd>
            <dt>{MOD}G</dt><dd>タグを付ける / 外す・新規作成</dd>
            <dt>{MOD}D</dt><dd>お気に入り</dd>
            <dt>{MOD}T</dt><dd>翻訳 / 元に戻す・画像は文字起こし</dd>
            <dt>{MOD}E / {MOD}N</dt><dd>定型文にする / 新しい定型文</dd>
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
        <input bind:value={editor.title} placeholder="タイトル（例: 住所、署名）" autofocus />
        <textarea bind:value={editor.text} placeholder="定型文の本文"></textarea>
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
    --card-w: 236px;
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
  .pill .star { color: #fbbf24; display: flex; }
  .pill.on .star { color: #fff3b0; }
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
  /* 多段: その日の最初のカードの左上に日付ラベル */
  .day {
    position: absolute; z-index: 2; top: -9px; left: 10px;
    padding: 1px 8px; border-radius: 999px;
    background: var(--fg); color: #15161a;
    font-size: 10.5px; font-weight: 800; letter-spacing: 0.04em;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.35);
    pointer-events: none;
  }

  /* 日付の区切り: 1段は縦書きの細い列、多段は段をまたぐ見出し */
  .daysep {
    flex: none;
    display: flex; align-items: center; justify-content: center;
    width: 22px;
    color: var(--fg-dim); font-size: 11px; font-weight: 700; letter-spacing: 0.08em;
    border-left: 1px solid var(--line);
  }
  .daysep:first-child { border-left: 0; }
  .daysep span { writing-mode: vertical-rl; white-space: nowrap; }

  .newcard {
    height: 100%;
    display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px;
    border-radius: 14px;
    border: 1.5px dashed var(--line-strong);
    color: var(--fg-dim); font-size: 13px; font-weight: 600;
  }
  .newcard:hover { color: var(--fg); border-color: var(--accent); }
  .newcard small { font-family: var(--mono); opacity: 0.6; }
  .emptylist { margin: auto; color: var(--fg-dim); font-size: 13px; }

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
