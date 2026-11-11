<script lang="ts">
  import { onMount } from "svelte";
  import { api, isMac, TAG_COLORS, type Settings, type Tag } from "./api";
  import ShortcutInput from "./ShortcutInput.svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  let s = $state<Settings | null>(null);
  let tags = $state<Tag[]>([]);
  let errors = $state<string[]>([]);
  let saved = $state(false);
  let trusted = $state(true);
  let confirmClear = $state(false);
  let savedTimer: ReturnType<typeof setTimeout>;

  const LANGS = [
    ["ja", "日本語（日本語の文は英語へ）"],
    ["en", "英語"],
    ["zh", "中国語（簡体）"],
    ["ko", "韓国語"],
    ["fr", "フランス語"],
    ["de", "ドイツ語"],
    ["es", "スペイン語"],
  ];
  const SEPARATORS = [
    ["\n", "改行"],
    ["\n\n", "空行"],
    [" ", "スペース"],
    ["", "なし（そのままつなぐ）"],
    [", ", "カンマ"],
  ];

  async function save() {
    if (!s) return;
    errors = await api.saveSettings($state.snapshot(s));
    s = await api.getSettings();
    saved = true;
    clearTimeout(savedTimer);
    savedTimer = setTimeout(() => (saved = false), 2000);
  }

  let tagError = $state("");
  let pickingColor = $state<number | null>(null);

  async function removeTag(t: Tag) {
    await api.deleteTag(t.id);
    tags = await api.listTags();
  }

  async function renameTag(t: Tag, name: string) {
    tagError = "";
    if (!name.trim() || name.trim() === t.name) return;
    try {
      await api.updateTag(t.id, name, t.color);
    } catch (err) {
      tagError = String(err);
    }
    tags = await api.listTags();
  }

  async function recolor(t: Tag, color: string) {
    await api.updateTag(t.id, t.name, color);
    pickingColor = null;
    tags = await api.listTags();
  }

  async function clearAll() {
    await api.clearHistory();
    confirmClear = false;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.defaultPrevented || e.isComposing || e.keyCode === 229) return;
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (mod && e.key.toLowerCase() === "s") {
      e.preventDefault();
      save();
    } else if ((mod && e.key.toLowerCase() === "w") || e.key === "Escape") {
      e.preventDefault();
      getCurrentWindow().close();
    }
  }

  onMount(() => {
    (async () => {
      s = await api.getSettings();
      tags = await api.listTags();
      trusted = await api.accessibilityStatus();
    })();
    const t = setInterval(async () => (trusted = await api.accessibilityStatus()), 3000);
    return () => clearInterval(t);
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if s}
  <main>
    <h1>Clipo 設定</h1>

    {#if isMac && !trusted}
      <div class="notice">
        <div>
          <b>自動貼り付けが無効です</b>
          <p>「システム設定 → プライバシーとセキュリティ → アクセシビリティ」で Clipo をオンにしてください。それまではクリップボードにコピーするだけになります。</p>
        </div>
        <button class="primary" onclick={() => api.openAccessibility()}>設定を開く</button>
      </div>
    {/if}

    <section>
      <h2>ショートカット</h2>
      <div class="row">
        <span>履歴パネルを開く</span>
        <ShortcutInput bind:value={s.panel_shortcut} />
      </div>
      <div class="row">
        <span>範囲を選んで文字起こし</span>
        <ShortcutInput bind:value={s.ocr_shortcut} />
      </div>
      <div class="row">
        <label class="check"><input type="checkbox" bind:checked={s.screenshot_enabled} /> 範囲スクショ（画像）</label>
        <ShortcutInput bind:value={s.screenshot_shortcut} disabled={!s.screenshot_enabled} />
      </div>
      {#if isMac}
        <div class="row">
          <label class="check"><input type="checkbox" bind:checked={s.watch_screenshots} /> Mac 標準のスクショ（⌘⇧3 / ⌘⇧4 / ⌘⇧5）も履歴とクリップボードに入れる</label>
        </div>
      {/if}
      <p class="help">
        クリックしてキーを押すと登録されます。{isMac ? "⌘ / ⌃ / ⌥ / ⇧" : "Ctrl / Shift / Alt"} を素早く2回押すと「2回押し」で登録できます。Esc で取り消し、Delete で未設定にできます。
        {#if isMac}2回押しを他のアプリで使うにはアクセシビリティの許可が必要です。{/if}
        {#if !isMac}Screenpresso など他のスクショアプリを使う場合は、範囲スクショをオフにするか別のキーにしてください。{/if}
      </p>
    </section>

    <section>
      <h2>表示</h2>
      <div class="row">
        <span>カードの最大段数<small>枚数が少ないときは段を減らします。2段以上は縦にスクロール（↑↓ で段を移動）</small></span>
        <div class="seg">
          {#each [1, 2, 3] as n}
            <button class:on={s.panel_rows === n} onclick={() => (s!.panel_rows = n)}>{n}段</button>
          {/each}
        </div>
      </div>
      <div class="row">
        <label class="check"><input type="checkbox" bind:checked={s.group_by_date} /> カードを日付ごとに区切って表示</label>
      </div>
      <div class="row">
        <span>背景の透け具合<small>左ほど透ける（{s.panel_opacity}%）</small></span>
        <span class="slider">
          <span class="swatch"><i style:background="rgba(22, 23, 28, {s.panel_opacity / 100})"></i></span>
          <input type="range" min="0" max="95" step="5" bind:value={s.panel_opacity} />
        </span>
      </div>
    </section>

    <section>
      <h2>履歴</h2>
      <div class="row">
        <span>保存する件数<small>お気に入り・定型文は数えません</small></span>
        <input class="num" type="number" min="10" max="10000" step="10" bind:value={s.max_history} />
      </div>
      <div class="row">
        <span>まとめて貼り付けの区切り</span>
        <select bind:value={s.multi_separator}>
          {#each SEPARATORS as [v, label]}<option value={v}>{label}</option>{/each}
        </select>
      </div>
      <div class="row">
        <span>履歴を全部消す<small>お気に入り・定型文は残ります</small></span>
        {#if confirmClear}
          <span class="inline">
            <button onclick={() => (confirmClear = false)}>やめる</button>
            <button class="danger" onclick={clearAll}>消去する</button>
          </span>
        {:else}
          <button onclick={() => (confirmClear = true)}>消去…</button>
        {/if}
      </div>
    </section>

    <section>
      <h2>翻訳</h2>
      <div class="row">
        <span>翻訳サービス</span>
        <select bind:value={s.translate_provider}>
          <option value="google">Google 翻訳（キー不要）</option>
          <option value="deepl">DeepL（API キーが必要）</option>
        </select>
      </div>
      {#if s.translate_provider === "deepl"}
        <div class="row">
          <span>DeepL API キー</span>
          <input class="text" type="password" bind:value={s.deepl_api_key} placeholder="xxxxxxxx-xxxx-…:fx" />
        </div>
      {:else}
        <p class="help">Google 翻訳はキー不要の非公式な方法で呼び出すため、将来使えなくなる可能性があります。安定して使うなら DeepL（無料枠 月50万文字）がおすすめです。</p>
      {/if}
      <div class="row">
        <span>訳す言語</span>
        <select bind:value={s.translate_target}>
          {#each LANGS as [v, label]}<option value={v}>{label}</option>{/each}
        </select>
      </div>
    </section>

    <section>
      <h2>タグ</h2>
      {#if tags.length}
        {#each tags as t (t.id)}
          <div class="row tagrow">
            <span class="inline">
              <button class="dotbtn" title="色を変更" onclick={() => (pickingColor = pickingColor === t.id ? null : t.id)}>
                <i style:background={t.color}></i>
              </button>
              <input
                class="text tagname"
                value={t.name}
                onkeydown={(e) => { if (e.key === "Enter" && !e.isComposing && e.keyCode !== 229) (e.currentTarget as HTMLInputElement).blur(); }}
                onchange={(e) => renameTag(t, (e.currentTarget as HTMLInputElement).value)}
              />
            </span>
            <button class="danger-ghost" onclick={() => removeTag(t)}>削除</button>
          </div>
          {#if pickingColor === t.id}
            <div class="palette">
              {#each TAG_COLORS as c}
                <button class="dotbtn" class:on={t.color === c} title={c} onclick={() => recolor(t, c)}><i style:background={c}></i></button>
              {/each}
            </div>
          {/if}
        {/each}
        {#if tagError}<p class="errors">{tagError}</p>{/if}
        <p class="help">名前を書き換えて Enter で変更。● を押すと色を選べます。タグを削除してもカードは残ります。</p>
      {:else}
        <p class="help">タグはパネル上部の「＋」から追加できます。</p>
      {/if}
    </section>

    <section>
      <h2>一般</h2>
      <div class="row">
        <label class="check"><input type="checkbox" bind:checked={s.launch_at_login} /> ログイン時に起動</label>
      </div>
    </section>

    <footer>
      {#if errors.length}
        <ul class="errors">{#each errors as err}<li>{err}</li>{/each}</ul>
      {:else if saved}
        <span class="ok">保存しました</span>
      {/if}
      <span class="keyhint">{isMac ? "⌘S" : "Ctrl+S"} で保存 / Esc で閉じる</span>
      <button class="primary" onclick={save}>保存</button>
    </footer>
  </main>
{/if}

<style>
  main { max-width: 560px; margin: 0 auto; padding: 22px 24px 90px; }
  h1 { font-size: 20px; margin: 0 0 16px; }
  h2 { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--fg-dim); margin: 0 0 6px; }
  section { margin-bottom: 18px; padding: 10px 14px; border-radius: 12px; background: var(--surface); border: 1px solid var(--line); }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 42px; }
  .row + .row { border-top: 1px solid var(--line); }
  .row > span { display: flex; flex-direction: column; font-size: 13.5px; min-width: 0; }
  .row > :last-child:not(:first-child) { flex: none; }
  button { white-space: nowrap; }
  small { font-size: 11.5px; color: var(--fg-dim); margin-top: 1px; }
  .help { margin: 6px 0 4px; font-size: 12px; line-height: 1.55; color: var(--fg-dim); }
  .check { display: flex; align-items: center; gap: 8px; font-size: 13.5px; }
  input.num { width: 100px; }
  input.text { width: 260px; }
  input.num, input.text, select {
    height: 32px; padding: 0 10px; border-radius: 8px;
    border: 1px solid var(--line-strong); background: var(--field); color: var(--fg); font: inherit; font-size: 13px;
  }
  button { height: 30px; padding: 0 12px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--field); color: var(--fg); font-size: 13px; }
  button:hover { border-color: var(--accent); }
  .primary { background: var(--accent); border-color: transparent; color: #fff; font-weight: 700; }
  .danger { background: #dc2626; border-color: transparent; color: #fff; }
  .inline { display: flex; gap: 6px; flex-direction: row !important; }
  .notice {
    display: flex; gap: 14px; align-items: center; margin-bottom: 18px; padding: 12px 14px;
    border-radius: 12px; background: color-mix(in srgb, #f59e0b 16%, var(--surface)); border: 1px solid color-mix(in srgb, #f59e0b 50%, transparent);
  }
  .notice p { margin: 4px 0 0; font-size: 12px; line-height: 1.5; color: var(--fg-dim); }
  .notice button { flex: none; }
  .tagrow .inline { align-items: center; }
  .tagname { width: 220px !important; }
  .dotbtn { width: 28px; height: 28px; padding: 0; display: grid; place-items: center; border-radius: 50%; }
  .dotbtn i { width: 14px; height: 14px; border-radius: 50%; display: block; }
  .dotbtn.on { border-color: var(--accent); box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 35%, transparent); }
  .palette { display: flex; gap: 6px; padding: 0 0 8px 34px; }
  .danger-ghost { color: #dc2626; }
  footer {
    position: fixed; left: 0; right: 0; bottom: 0;
    display: flex; align-items: center; justify-content: flex-end; gap: 12px;
    padding: 12px 24px; background: var(--bg); border-top: 1px solid var(--line);
  }
  footer .primary { height: 34px; padding: 0 22px; }
  .seg { flex: none; display: flex; border: 1px solid var(--line-strong); border-radius: 8px; overflow: hidden; }
  .seg button { border: 0; border-radius: 0; height: 30px; padding: 0 14px; white-space: nowrap; }
  .seg button + button { border-left: 1px solid var(--line-strong); }
  .seg button.on { background: var(--accent); color: #fff; font-weight: 700; }
  .slider { display: flex !important; flex-direction: row !important; align-items: center; gap: 10px; }
  .slider input { width: 180px; accent-color: var(--accent); }
  .swatch i { display: block; width: 100%; height: 100%; }
  .swatch {
    overflow: hidden;
    width: 34px; height: 22px; border-radius: 6px; border: 1px solid var(--line-strong);
    background-image: linear-gradient(45deg, #bbb 25%, transparent 25%, transparent 75%, #bbb 75%),
      linear-gradient(45deg, #bbb 25%, transparent 25%, transparent 75%, #bbb 75%);
  }
  .keyhint { font-size: 11.5px; color: var(--fg-dim); margin-right: auto; }
  .ok { color: #16a34a; font-size: 13px; }
  .errors { margin: 0; padding: 0; list-style: none; color: #dc2626; font-size: 12px; flex: 1; }
</style>
