<script lang="ts" module>
  export type MenuEntry =
    | {
        kind?: "item";
        label: string;
        icon?: string;
        /** 先頭に表示する色つきの点（タグ用） */
        dot?: string;
        img?: string;
        checked?: boolean;
        hint?: string;
        count?: number;
        danger?: boolean;
        /** true なら実行後もメニューを閉じない（タグのオン/オフなど） */
        keepOpen?: boolean;
        run: () => void | Promise<void>;
      }
    | { kind: "sep"; label?: string }
    /** 1 行に並べる色見本。←→ で選んで Enter */
    | { kind: "swatches"; label: string; colors: string[]; value: string; onpick: (c: string) => void | Promise<void> };
</script>

<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    entries,
    x,
    y,
    width = 230,
    onclose,
  }: { entries: MenuEntry[]; x: number; y: number; width?: number; onclose: () => void } = $props();

  let active = $state(0);
  /** 色見本の行で選んでいる色 */
  let swatch = $state(0);
  // 色見本の行に来たら、いまの色から選び始める
  $effect(() => {
    const cur = entries[active];
    if (cur?.kind === "swatches") swatch = Math.max(0, cur.colors.indexOf(cur.value));
  });
  let el: HTMLDivElement;
  // 実際の大きさを測って、ウィンドウからはみ出さない位置に置く
  let pos = $state({ left: 0, top: 0 });
  $effect(() => {
    const w = el?.offsetWidth ?? width;
    const h = el?.offsetHeight ?? 0;
    entries; // エントリが変わったら測り直す
    pos = {
      left: Math.max(6, Math.min(x, window.innerWidth - w - 6)),
      top: Math.max(6, Math.min(y, window.innerHeight - h - 6)),
    };
  });

  const runnable = $derived(
    entries.map((e, i) => (e.kind === "sep" ? -1 : i)).filter((i) => i >= 0),
  );

  $effect(() => {
    // エントリが変わっても選択位置を範囲内に保つ
    if (!runnable.includes(active)) active = runnable[0] ?? 0;
  });

  async function scrollActive() {
    await tick();
    el?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }

  async function run(i: number) {
    const e = entries[i];
    if (!e || e.kind === "sep") return;
    if (e.kind === "swatches") {
      await e.onpick(e.colors[swatch]);
      return;
    }
    await e.run();
    if (!e.keepOpen) onclose();
  }

  /** パネル側のキー処理から呼ぶ。処理したら true */
  export function handleKey(e: KeyboardEvent): boolean {
    const pos = runnable.indexOf(active);
    const cur = entries[active];
    if (cur?.kind === "swatches" && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      const n = cur.colors.length;
      swatch = (swatch + (e.key === "ArrowRight" ? 1 : -1) + n) % n;
      e.preventDefault();
      return true;
    }
    switch (e.key) {
      case "ArrowDown":
        active = runnable[(pos + 1) % runnable.length];
        break;
      case "ArrowUp":
        active = runnable[(pos - 1 + runnable.length) % runnable.length];
        break;
      case "Home":
        active = runnable[0];
        break;
      case "End":
        active = runnable[runnable.length - 1];
        break;
      case "Enter":
      case " ":
        run(active);
        break;
      case "Escape":
      case "ArrowLeft":
      case "Tab":
        onclose();
        break;
      default: {
        // 頭文字で項目にジャンプ
        if (e.key.length === 1 && !e.metaKey && !e.ctrlKey) {
          const k = e.key.toLowerCase();
          const start = pos + 1;
          for (let n = 0; n < runnable.length; n++) {
            const idx = runnable[(start + n) % runnable.length];
            const it = entries[idx];
            if (it.kind !== "sep" && it.label.toLowerCase().startsWith(k)) {
              active = idx;
              break;
            }
          }
          break;
        }
        return false;
      }
    }
    e.preventDefault();
    scrollActive();
    return true;
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div
  class="menu"
  bind:this={el}
  style:left="{pos.left}px"
  style:top="{pos.top}px"
  style:width="{width}px"
  role="menu"
  tabindex="-1"
  onclick={(e) => e.stopPropagation()}
>
  {#each entries as e, i}
    {#if e.kind === "sep"}
      <div class="sep">{e.label ?? ""}</div>
    {:else if e.kind === "swatches"}
      <div
        class="swatches"
        class:active={i === active}
        data-i={i}
        role="menuitem"
        tabindex="-1"
        onmouseenter={() => (active = i)}
      >
        <span class="label">{e.label}</span>
        {#each e.colors as c, n}
          <button
            class="sw"
            class:cur={c === e.value}
            class:kbd={i === active && n === swatch}
            style:background={c}
            title={c}
            aria-label={c}
            onclick={() => { swatch = n; e.onpick(c); }}
          ></button>
        {/each}
      </div>
    {:else}
      <button
        data-i={i}
        role="menuitem"
        class:active={i === active}
        class:danger={e.danger}
        onmouseenter={() => (active = i)}
        onclick={() => run(i)}
      >
        {#if e.checked !== undefined}<span class="tick">{e.checked ? "✓" : ""}</span>{/if}
        {#if e.img}<img src={e.img} alt="" />
        {:else if e.dot}<i style:background={e.dot}></i>
        {:else if e.icon}<Icon name={e.icon} size={14} />{/if}
        <span class="label">{e.label}</span>
        {#if e.count !== undefined}<span class="hint">{e.count}</span>{/if}
        {#if e.hint}<span class="hint">{e.hint}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed; z-index: 30;
    max-height: calc(100vh - 12px); overflow-y: auto;
    padding: 4px;
    border-radius: 11px;
    background: var(--menu);
    border: 1px solid var(--glass-border);
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.4);
    outline: none;
  }
  button {
    width: 100%; display: flex; align-items: center; gap: 8px;
    padding: 4px 8px; border-radius: 6px; font-size: 12.5px; line-height: 1.35; text-align: left; color: var(--fg);
  }
  button.active { background: var(--accent); color: #fff; }
  .danger { color: #f87171; }
  .danger.active { background: #dc2626; color: #fff; }
  .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hint { font: 500 11px/1 var(--mono); opacity: 0.6; }
  .tick { width: 12px; font-size: 11px; }
  i { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  img { width: 16px; height: 16px; flex: none; }
  .swatches {
    display: flex; align-items: center; gap: 6px;
    padding: 5px 8px; border-radius: 6px; font-size: 12.5px; color: var(--fg);
  }
  .swatches.active { background: var(--hover, rgba(255, 255, 255, 0.08)); }
  .swatches .label { flex: none; margin-right: 2px; }
  .sw {
    width: 16px; height: 16px; padding: 0; flex: none;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.25);
  }
  .sw.cur { box-shadow: 0 0 0 2px var(--menu), 0 0 0 4px #fff; }
  .sw.kbd { transform: scale(1.25); }
  .sw:hover { transform: scale(1.2); }
  .sep { margin: 3px 8px 1px; padding-top: 3px; border-top: 1px solid var(--line); font-size: 10.5px; color: var(--fg-dim); }
  .sep:empty { padding-top: 0; margin: 4px 8px; }
</style>
