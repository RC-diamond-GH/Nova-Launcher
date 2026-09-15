/** Status texts shown on the home page and under the java path field. */

import { detectHint, javaInfo, statusEl } from "./dom";

export function setStatus(text: string, cls: "idle" | "running" | "error"): void {
  statusEl.textContent = text;
  statusEl.className = `status ${cls}`;
}

export function setJavaInfo(text: string, bad: boolean): void {
  javaInfo.textContent = text;
  javaInfo.classList.toggle("bad", bad);
}

/** `detail` becomes the hover tooltip; `bad` only warns about the text itself. */
export function setDetectHint(text: string, detail: string, bad = false): void {
  detectHint.textContent = text;
  detectHint.title = detail;
  detectHint.classList.toggle("bad", bad);
}
