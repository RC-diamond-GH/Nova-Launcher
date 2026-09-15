/**
 * Typed access to the elements declared in `index.html`; this is the single
 * place where DOM ids appear as strings.
 */

export function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`index.html is missing #${id}`);
  return node as T;
}

/* 主页 */
export const avatar = el<HTMLElement>("avatar");
export const playerName = el<HTMLElement>("player-name");
export const statusEl = el<HTMLElement>("status");
export const detectHint = el<HTMLElement>("detect-hint");
export const launchBtn = el<HTMLButtonElement>("launch");

/* 设置 */
export const usernameInput = el<HTMLInputElement>("username");
export const javaPathInput = el<HTMLInputElement>("java-path");
export const javaInfo = el<HTMLElement>("java-info");
export const memRange = el<HTMLInputElement>("mem-range");
export const memNum = el<HTMLInputElement>("mem-num");
export const memLabel = el<HTMLElement>("mem-label");

/* 日志 */
export const logsEnabled = el<HTMLInputElement>("logs-enabled");
export const consoleEl = el<HTMLElement>("console");
