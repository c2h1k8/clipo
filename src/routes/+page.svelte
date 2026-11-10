<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Panel from "$lib/Panel.svelte";
  import SettingsView from "$lib/SettingsView.svelte";
  import "$lib/global.css";

  const label = getCurrentWindow().label;
  if (label === "panel") document.documentElement.classList.add("is-panel");
</script>

{#if label === "settings"}
  <SettingsView />
{:else}
  <Panel />
{/if}
