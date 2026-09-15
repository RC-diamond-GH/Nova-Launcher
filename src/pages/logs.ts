/** Logs page: the capture toggle, the live console widget, and log export. */

import * as backend from "../api/backend";
import { scheduleSave } from "../config";
import { consoleEl, el, logsEnabled } from "../ui/dom";
import { setStatus } from "../ui/status";

/** Keeps the DOM bounded while the game streams; the file export gets all of it. */
const MAX_LOG_LINES = 2000;
/** Pixels of slack before autoscroll disengages. */
const FOLLOW_SLACK_PX = 24;

let stickToBottom = true;

export function appendLog(line: string): void {
  consoleEl.appendChild(document.createTextNode(line + "\n"));
  while (consoleEl.childNodes.length > MAX_LOG_LINES) {
    consoleEl.removeChild(consoleEl.firstChild!);
  }
  if (stickToBottom) {
    consoleEl.scrollTop = consoleEl.scrollHeight;
  }
}

export function initLogsPage(): void {
  // The toggle lives here but persists with the rest of the config.
  logsEnabled.addEventListener("change", scheduleSave);
  el<HTMLButtonElement>("export-log").addEventListener("click", () => void exportLog());
  consoleEl.addEventListener("scroll", () => {
    stickToBottom =
      consoleEl.scrollHeight - consoleEl.scrollTop - consoleEl.clientHeight < FOLLOW_SLACK_PX;
  });
}

async function exportLog(): Promise<void> {
  const content = consoleEl.textContent ?? "";
  if (!content.trim()) {
    setStatus("日志为空，无可导出", "error");
    return;
  }
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
  const path = await backend.pickLogPath(`nova-launcher-${stamp}.log`);
  if (!path) return;
  try {
    await backend.writeLogFile(path, content);
    setStatus("日志已导出", "idle");
  } catch (e) {
    setStatus(String(e), "error");
  }
}
