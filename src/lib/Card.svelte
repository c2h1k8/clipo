<script lang="ts">
  import { fileSrc, timeAgo, type Item, type Tag } from "./api";
  import Icon from "./Icon.svelte";
  import { untrack } from "svelte";

  let {
    item,
    tags,
    index,
    selected,
    focused,
    now,
    busy = false,
    order = null,
    dimmed = false,
    onclick,
    ondblclick,
    oncontextmenu,
    onaction,
    onrevert,
  }: {
    item: Item;
    tags: Tag[];
    index: number;
    selected: boolean;
    focused: boolean;
    now: number;
    busy?: boolean;
    /** 複数選択時の選んだ順番（1始まり） */
    order?: number | null;
    dimmed?: boolean;
    onclick: (e: MouseEvent) => void;
    ondblclick: () => void;
    oncontextmenu: (e: MouseEvent) => void;
    onaction: () => void;
    onrevert: () => void;
  } = $props();

  // 翻訳の切り替えはカードを裏返すアニメーションで見せる。
  // 表示内容は回転の途中（真横を向いた瞬間）で入れ替える
  let translated = $state(untrack(() => item.translated != null));
  let shownTranslation = $state(untrack(() => item.translated));
  let flip = $state<"" | "flip-in" | "flip-out">("");
  let flipTimers: ReturnType<typeof setTimeout>[] = [];
  $effect(() => {
    const next = item.translated != null;
    const text = item.translated;
    if (next === translated) {
      if (next) shownTranslation = text;
      return;
    }
    flipTimers.forEach(clearTimeout);
    flip = next ? "flip-in" : "flip-out";
    flipTimers = [
      setTimeout(() => {
        translated = next;
        if (next) shownTranslation = text;
      }, 260),
      setTimeout(() => (flip = ""), 580),
    ];
  });
  const variant = $derived(item.snippet ? "snippet" : translated ? "translated" : item.kind);
  const label = $derived(
    item.snippet ? (item.title || "定型文") : translated ? "翻訳済み" : item.kind === "image" ? "画像" : "テキスト",
  );
  const itemTags = $derived(tags.filter((t) => item.tags.includes(t.id)));
  const initials = $derived((item.source_name ?? "?").slice(0, 2).toUpperCase());
  let iconFailed = $state(false);
  const actionTitle = $derived(item.kind === "image" ? "文字起こし" : "翻訳");
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="card {variant} {flip}"
  class:selected
  class:focused
  class:dimmed
  role="option"
  tabindex="-1"
  aria-selected={selected}
  data-index={index}
  {onclick}
  {ondblclick}
  {oncontextmenu}
>
  <header>
    <span class="kind">
      {#if order}<span class="order">{order}</span>{/if}
      {#if translated}
        <span class="check"><Icon name="check" size={11} /></span>
      {:else}
        <Icon name={item.snippet ? "snippet" : item.kind === "image" ? "image" : "text"} size={14} />
      {/if}
      <span class="label">{label}</span>
    </span>
    <span class="right">
      {#if item.favorite}<span class="star"><Icon name="star-fill" size={13} /></span>{/if}
      {#each itemTags.slice(0, 2) as t (t.id)}
        <span class="chip"><i style:background={t.color}></i>{t.name}</span>
      {/each}
      {#if itemTags.length > 2}<span class="chip">+{itemTags.length - 2}</span>{/if}
      {#if translated}
        <button class="ghost" title="元の文に戻す" onclick={(e) => { e.stopPropagation(); onrevert(); }}>
          <Icon name="globe" size={14} />
        </button>
      {/if}
      {#if index < 9}<kbd>{index + 1}</kbd>{/if}
    </span>
  </header>

  <div class="body">
    {#if item.kind === "image"}
      <img src={fileSrc(item.thumb_path)} alt="" loading="lazy" draggable="false" />
    {:else}
      <p>{translated ? shownTranslation : item.preview}</p>
    {/if}
  </div>

  <footer>
    <span class="src" title={item.source_name ?? "不明"}>
      {#if item.source_id === "clipo.screenshot"}
        <span class="badge sys"><Icon name="camera" size={11} /></span>
      {:else if item.source_id === "clipo.ocr"}
        <span class="badge sys"><Icon name="scan" size={11} /></span>
      {:else if item.source_icon && !iconFailed}
        <img class="appicon" src={fileSrc(item.source_icon)} alt="" onerror={() => (iconFailed = true)} />
      {:else if item.source_name}
        <span class="badge">{initials}</span>
      {/if}
      <span class="meta">
        {item.snippet ? "定型文" : timeAgo(item.used_at, now)}
        {#if item.kind === "image" && item.width}· {item.width}×{item.height}{/if}
        {#if item.kind === "text" && item.char_count > 0 && !translated}· {item.char_count}字{/if}
      </span>
    </span>
    {#if !translated}
      <button
        class="action"
        class:spin={busy}
        title={actionTitle}
        onclick={(e) => { e.stopPropagation(); onaction(); }}
      >
        <Icon name={busy ? "loader" : item.kind === "image" ? "scan" : "globe"} size={15} />
      </button>
    {/if}
  </footer>
</div>

<style>
  .card {
    --head: var(--blue);
    --tint: #eef5ff;
    --ink: #1d2433;
    flex: 0 0 var(--card-w);
    height: 100%;
    display: flex;
    flex-direction: column;
    border-radius: 14px;
    overflow: hidden;
    background: var(--tint);
    color: var(--ink);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.18), 0 6px 18px rgba(0, 0, 0, 0.12);
    --ring: #4da3ff;
    --ring-ink: #1d6fe0;
    outline: 3px solid transparent;
    outline-offset: 0;
    transition: transform 0.16s ease, outline-color 0.12s ease, box-shadow 0.16s ease, opacity 0.16s ease, filter 0.16s ease;
    cursor: default;
    user-select: none;
    position: relative;
  }
  .card:hover { transform: translateY(-2px); }
  .card.dimmed { opacity: 0.5; filter: saturate(0.6); }
  .card.dimmed:hover { opacity: 0.8; }
  .card.selected {
    outline: 3px solid var(--ring);
    box-shadow: 0 0 0 6px color-mix(in srgb, var(--ring) 28%, transparent), 0 10px 24px rgba(0, 0, 0, 0.3);
  }
  /* いま操作しているカード（矢印キーの位置）は浮き上がらせる */
  .card.focused {
    transform: translateY(-6px);
    outline: 3px solid var(--ring);
    box-shadow: 0 0 0 6px color-mix(in srgb, var(--ring) 35%, transparent), 0 16px 30px rgba(0, 0, 0, 0.38);
  }
  .card.focused:not(.selected) { outline-style: dashed; }

  .order {
    min-width: 20px; height: 20px; padding: 0 5px;
    display: grid; place-items: center;
    border-radius: 999px;
    background: #fff; color: var(--ring-ink);
    font: 800 11.5px/1 var(--mono);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  }

  .flip-in { animation: flip-in 0.58s cubic-bezier(0.45, 0, 0.25, 1); }
  .flip-out { animation: flip-out 0.58s cubic-bezier(0.45, 0, 0.25, 1); }
  @keyframes flip-in {
    0% { transform: perspective(900px) rotateY(0deg); }
    45% { transform: perspective(900px) rotateY(90deg) scale(0.96); }
    55% { transform: perspective(900px) rotateY(-90deg) scale(0.96); }
    100% { transform: perspective(900px) rotateY(0deg); }
  }
  @keyframes flip-out {
    0% { transform: perspective(900px) rotateY(0deg); }
    45% { transform: perspective(900px) rotateY(-90deg) scale(0.96); }
    55% { transform: perspective(900px) rotateY(90deg) scale(0.96); }
    100% { transform: perspective(900px) rotateY(0deg); }
  }
  @media (prefers-reduced-motion: reduce) {
    .flip-in, .flip-out { animation: none; }
  }

  .image { --head: var(--orange); --tint: #fff6ee; }
  .snippet { --head: var(--green); --tint: #eefaf4; }
  .translated { --head: #2f2a63; --tint: #1e1b3d; --ink: #f1efff; }

  header {
    height: 34px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 0 8px 0 12px;
    background: var(--head);
    color: #fff;
    font-weight: 700;
    font-size: 13px;
  }
  .kind { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .label { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .check { display: grid; place-items: center; width: 16px; height: 16px; border-radius: 50%; background: #22c55e; color: #fff; }
  .right { display: flex; align-items: center; gap: 4px; flex: none; }
  .star { color: #ffd84d; display: flex; }
  .chip {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 2px 7px; border-radius: 999px;
    background: rgba(255, 255, 255, 0.92); color: #333;
    font-size: 11px; font-weight: 600; max-width: 76px;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .chip i { width: 7px; height: 7px; border-radius: 50%; flex: none; }
  kbd {
    font: 600 10px/1 var(--mono);
    color: rgba(255, 255, 255, 0.75);
    padding: 3px 5px; border-radius: 5px;
    background: rgba(0, 0, 0, 0.16);
  }
  .ghost { color: #cfc8ff; display: flex; padding: 2px; border-radius: 6px; }
  .ghost:hover { background: rgba(255, 255, 255, 0.12); color: #fff; }

  .body { flex: 1; min-height: 0; padding: 10px 12px 4px; overflow: hidden; }
  .body p {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    display: -webkit-box;
    -webkit-line-clamp: 6;
    line-clamp: 6;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .body img { width: 100%; height: 100%; object-fit: contain; display: block; }

  footer {
    height: 36px; flex: none;
    display: flex; align-items: center; justify-content: space-between;
    padding: 0 8px 0 12px;
  }
  .src { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .appicon { width: 16px; height: 16px; flex: none; }
  .badge {
    width: 18px; height: 18px; border-radius: 50%; flex: none;
    display: grid; place-items: center;
    font-size: 8px; font-weight: 800; color: #fff; background: #64748b;
  }
  .badge.sys { background: var(--accent); }
  .meta { font-size: 11px; opacity: 0.55; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .action {
    width: 28px; height: 28px; border-radius: 50%; flex: none;
    display: grid; place-items: center;
    background: var(--accent); color: #fff;
    opacity: 0;
    transition: opacity 0.12s, transform 0.12s;
  }
  .card:hover .action, .card.selected .action, .action.spin { opacity: 1; }
  .action:hover { transform: scale(1.08); }
  .image .action { background: var(--orange); }
  .spin :global(svg) { animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
