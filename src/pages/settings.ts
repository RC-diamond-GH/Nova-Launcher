/** Settings page: the config form, its autosave, and the java probe. */

import * as backend from "../api/backend";
import { currentConfig, scheduleSave } from "../config";
import { el, javaPathInput, memNum, memRange, usernameInput } from "../ui/dom";
import { setMemory } from "../ui/memory";
import { syncProfile } from "../ui/profile";
import { setJavaInfo } from "../ui/status";

export function initSettingsPage(): void {
  usernameInput.addEventListener("input", () => {
    syncProfile();
    scheduleSave();
  });
  javaPathInput.addEventListener("input", () => {
    setJavaInfo("", false);
    scheduleSave();
  });
  memRange.addEventListener("input", () => {
    setMemory(Number(memRange.value));
    scheduleSave();
  });
  memNum.addEventListener("input", () => {
    setMemory(memNum.valueAsNumber);
    scheduleSave();
  });
  el<HTMLButtonElement>("java-browse").addEventListener("click", () => void browseJava());
  el<HTMLButtonElement>("java-check").addEventListener("click", () => void checkJava());
}

async function browseJava(): Promise<void> {
  const picked = await backend.pickJavaExecutable();
  if (!picked) return;
  javaPathInput.value = picked;
  setJavaInfo("", false);
  scheduleSave();
}

async function checkJava(): Promise<void> {
  const path = javaPathInput.value.trim();
  if (!path) {
    setJavaInfo("请先填写 Java 路径", true);
    return;
  }
  setJavaInfo("检测中…", false);
  try {
    const info = await backend.validateJava(path);
    let text = `Java ${info.version}（${info.is_64bit ? "64 位" : "32 位"}）`;
    const over32bit = !info.is_64bit && currentConfig().memory_mb > 1536;
    if (over32bit) text += " ⚠ 32 位 Java 分配超过 1536 MB 可能无法启动";
    setJavaInfo(text, over32bit);
  } catch (e) {
    setJavaInfo(`无法运行：${e}`, true);
  }
}
