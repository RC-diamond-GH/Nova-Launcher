/** Composition root: builds each page, restores the saved config, detects the setup. */

import { loadSavedConfig } from "./config";
import { initHomePage, refreshDetect } from "./pages/home";
import { initLogsPage } from "./pages/logs";
import { initSettingsPage } from "./pages/settings";
import { initNav } from "./ui/nav";
import { syncProfile } from "./ui/profile";

async function init(): Promise<void> {
  initNav();
  initSettingsPage();
  initLogsPage();
  await initHomePage();

  await loadSavedConfig();
  syncProfile();
  await refreshDetect();
}

window.addEventListener("DOMContentLoaded", () => void init());
