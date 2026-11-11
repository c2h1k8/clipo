<script lang="ts">
  import { api, isMac, prettyShortcut } from "./api";

  let { value = $bindable(), disabled = false }: { value: string; disabled?: boolean } = $props();
  let recording = $state(false);

  const KEYMAP: Record<string, string> = {
    Space: "Space", Enter: "Enter", Tab: "Tab", Backspace: "Backspace", Delete: "Delete",
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
    Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/", Semicolon: ";",
    Quote: "'", BracketLeft: "[", BracketRight: "]", Backslash: "\\", Backquote: "`",
    Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown", Insert: "Insert",
  };

  function keyName(code: string): string | null {
    if (/^Key[A-Z]$/.test(code)) return code.slice(3);
    if (/^Digit[0-9]$/.test(code)) return code.slice(5);
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
    if (/^Numpad[0-9]$/.test(code)) return "Num" + code.slice(6);
    return KEYMAP[code] ?? null;
  }

  // 修飾キーだけの 2 回押し（例: Ctrl を素早く 2 回）の検出
  const MOD_CODES: Record<string, string> = {
    ControlLeft: "Control", ControlRight: "Control",
    ShiftLeft: "Shift", ShiftRight: "Shift",
    AltLeft: "Alt", AltRight: "Alt",
    // Windows キーはスタートメニューが開くので Mac の ⌘ のみ
    ...(isMac ? { MetaLeft: "Command", MetaRight: "Command" } : {}),
  };
  let held: { mod: string; at: number; clean: boolean } | null = null;
  let lastTap: { mod: string; at: number } | null = null;

  async function start(e: MouseEvent) {
    e.stopPropagation();
    if (disabled || recording) return;
    recording = true;
    await api.pauseShortcuts(true);
  }

  // macOS の WebView ではクリックしてもボタンにフォーカスが移らないため、
  // 記録中はウィンドウ全体でキーを受け取る
  $effect(() => {
    if (!recording) return;
    held = null;
    lastTap = null;
    const onKey = (e: KeyboardEvent) => onKeydown(e);
    const onUp = (e: KeyboardEvent) => onKeyup(e);
    const onClickOutside = () => stop();
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onUp, true);
    window.addEventListener("click", onClickOutside);
    window.addEventListener("blur", onClickOutside);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onUp, true);
      window.removeEventListener("click", onClickOutside);
      window.removeEventListener("blur", onClickOutside);
    };
  });

  async function stop() {
    if (!recording) return;
    recording = false;
    await api.pauseShortcuts(false);
  }

  function onKeydown(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    const mod = MOD_CODES[e.code];
    if (mod) {
      if (e.repeat) return;
      if (held) {
        held.clean = false; // 修飾キーの同時押し
        lastTap = null;
      } else {
        held = { mod, at: performance.now(), clean: true };
      }
      return;
    }
    if (held) held.clean = false;
    lastTap = null;
    if (e.key === "Escape") return stop();
    if ((e.key === "Backspace" || e.key === "Delete") && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
      value = "";
      return stop();
    }
    const key = keyName(e.code);
    if (!key) return; // 修飾キーだけ押している途中
    const mods: string[] = [];
    if (isMac) {
      if (e.metaKey) mods.push("CommandOrControl");
      if (e.ctrlKey) mods.push("Control");
    } else {
      if (e.ctrlKey) mods.push("CommandOrControl");
      if (e.metaKey) mods.push("Super");
    }
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (!mods.length && !/^F\d+$/.test(key)) return; // 修飾キーなしは F キーのみ許可
    value = [...mods, key].join("+");
    stop();
  }

  function onKeyup(e: KeyboardEvent) {
    if (!recording) return;
    const mod = MOD_CODES[e.code];
    if (!mod || !held || held.mod !== mod) {
      if (!mod) return;
      held = null;
      return;
    }
    e.preventDefault();
    const now = performance.now();
    const tap = held.clean && now - held.at < 350;
    held = null;
    if (!tap) {
      lastTap = null;
      return;
    }
    if (lastTap && lastTap.mod === mod && now - lastTap.at < 500) {
      value = `DoubleTap+${mod}`;
      stop();
    } else {
      lastTap = { mod, at: now };
    }
  }
</script>

<button
  type="button"
  class="sc"
  class:recording
  {disabled}
  onclick={start}
>
  {#if recording}
    キーを押してください…
  {:else}
    <span class="keys">{prettyShortcut(value)}</span>
  {/if}
</button>

<style>
  .sc {
    min-width: 170px; height: 32px; padding: 0 12px;
    border-radius: 8px;
    border: 1px solid var(--line-strong);
    background: var(--field);
    color: var(--fg);
    font-size: 13px;
    text-align: center;
  }
  .sc:hover:not(:disabled) { border-color: var(--accent); }
  .sc:disabled { opacity: 0.45; }
  .keys { font-family: var(--mono); font-weight: 600; letter-spacing: 0.04em; }
  .recording {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 30%, transparent);
    color: var(--accent);
  }
</style>
