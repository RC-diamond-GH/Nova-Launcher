/**
 * The settings form as the launcher's config: snapshot, debounced persistence,
 * and population from a saved file.
 */

import * as backend from "./api/backend";
import type { LauncherConfig } from "./api/types";
import { javaPathInput, logsEnabled, memNum, usernameInput } from "./ui/dom";
import { clampMemory, setMemory } from "./ui/memory";

/** Long enough to absorb typing bursts, short enough to survive a quick close. */
const SAVE_DEBOUNCE_MS = 400;

let saveTimer: number | undefined;

export function currentConfig(): LauncherConfig {
  return {
    java_path: javaPathInput.value.trim(),
    username: usernameInput.value.trim(),
    memory_mb: clampMemory(memNum.valueAsNumber),
    logs_enabled: logsEnabled.checked,
  };
}

export function scheduleSave(): void {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    backend
      .saveConfig(currentConfig())
      .catch((e) => console.error("save_config failed:", e));
  }, SAVE_DEBOUNCE_MS);
}

/** Fills the form from a persisted config; `null` means "backend defaults". */
export function applyConfig(cfg: LauncherConfig | null): void {
  javaPathInput.value = cfg?.java_path ?? "";
  usernameInput.value = cfg?.username || "Player";
  logsEnabled.checked = cfg?.logs_enabled ?? true;
  setMemory(cfg?.memory_mb ?? 2048);
}

export async function loadSavedConfig(): Promise<void> {
  try {
    applyConfig(await backend.loadConfig());
  } catch (e) {
    console.error("load_config failed:", e);
    applyConfig(null);
  }
}
