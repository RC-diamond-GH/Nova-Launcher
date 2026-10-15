/** Settings form, Java validation, and macOS runtime discovery. */

import * as backend from "../api/backend";
import type { JavaInfo, JavaInstallation } from "../api/types";
import { currentConfig, scheduleSave } from "../config";
import {
  el, javaPathInput, javaResults, javaScanButton, javaScanDetails,
  javaScanStatus, javaScanWarnings, memNum, memRange, usernameInput,
} from "../ui/dom";
import { setMemory } from "../ui/memory";
import { syncProfile } from "../ui/profile";
import { setJavaInfo } from "../ui/status";

let installations: JavaInstallation[] = [];
let probeRequest = 0;

export function initSettingsPage(): void {
  usernameInput.addEventListener("input", () => {
    syncProfile();
    scheduleSave();
  });
  javaPathInput.addEventListener("input", () => {
    probeRequest++;
    setJavaInfo("", false);
    renderJavaResults();
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
  javaScanButton.addEventListener("click", () => void scanJava());
}

async function browseJava(): Promise<void> {
  try {
    const picked = await backend.pickJavaExecutable();
    if (!picked) return;
    probeRequest++;
    javaPathInput.value = picked;
    setJavaInfo("", false);
    renderJavaResults();
    scheduleSave();
  } catch (e) {
    setJavaInfo(`无法选择 Java：${e}`, true);
  }
}

async function checkJava(): Promise<void> {
  const request = ++probeRequest;
  const path = javaPathInput.value.trim();
  if (!path) {
    setJavaInfo("请先填写 Java 路径", true);
    return;
  }
  setJavaInfo("检测中…", false);
  try {
    const info = await backend.validateJava(path);
    if (request !== probeRequest || javaPathInput.value.trim() !== path) return;
    showJavaInfo(info);
  } catch (e) {
    if (request !== probeRequest || javaPathInput.value.trim() !== path) return;
    setJavaInfo(`无法运行：${e}`, true);
  }
}

function showJavaInfo(info: JavaInfo): void {
  let text = `Java ${info.version} · ${info.architecture} · ${info.vendor}`;
  // Unknown bitness must not be reported as 32-bit.
  const over32bit = !info.is_64bit && info.architecture === "x86"
    && currentConfig().memory_mb > 1536;
  if (over32bit) text += " · 32 位 Java 分配超过 1536 MB 可能无法启动";
  setJavaInfo(text, over32bit);
}

export async function scanJava(): Promise<void> {
  if (javaScanButton.disabled) return;
  javaScanButton.disabled = true;
  javaScanStatus.classList.remove("bad");
  javaScanStatus.textContent = "正在寻找 Java 并读取版本和架构…";
  javaResults.setAttribute("aria-busy", "true");
  installations = [];
  renderJavaResults();
  javaScanWarnings.replaceChildren();
  javaScanDetails.hidden = true;
  javaScanDetails.open = false;
  try {
    const result = await backend.discoverJava();
    installations = result.installations;
    renderJavaResults();
    javaScanStatus.textContent = !result.supported
      ? "自动寻找目前仅支持 macOS；仍可手动浏览或填写 Java 路径。"
      : installations.length > 0
        ? `找到 ${installations.length} 个 Java。选用后自动保存，不会自动覆盖当前路径。`
        : "未找到可运行的 Java；可安装 JDK 后重新扫描，或手动填写路径并检测。";
    for (const warning of result.warnings) {
      const item = document.createElement("li");
      item.textContent = warning;
      javaScanWarnings.append(item);
    }
    javaScanDetails.hidden = result.warnings.length === 0;
  } catch (e) {
    javaScanStatus.textContent = `扫描失败：${e}。仍可手动填写路径或重新扫描。`;
    javaScanStatus.classList.add("bad");
  } finally {
    javaScanButton.disabled = false;
    javaResults.removeAttribute("aria-busy");
  }
}

function renderJavaResults(): void {
  const rows = installations.map((installation) => {
    const { path, info } = installation;
    const row = document.createElement("li");
    row.className = "java-result";
    const description = document.createElement("div");
    description.className = "java-result-description";
    const title = document.createElement("span");
    title.className = "java-result-title";
    title.textContent = `Java ${info.version} · ${info.architecture} · ${info.vendor}`;
    const executable = document.createElement("code");
    executable.className = "java-result-path";
    executable.textContent = path;
    description.append(title, executable);

    const selected = javaPathInput.value.trim() === path;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "btn small";
    button.textContent = selected ? "已选用" : "选用";
    button.disabled = selected;
    button.setAttribute("aria-label", `${selected ? "已选用" : "选用"} Java ${info.version} ${info.architecture}：${path}`);
    button.addEventListener("click", () => {
      probeRequest++;
      javaPathInput.value = path;
      showJavaInfo(info);
      scheduleSave();
      renderJavaResults();
    });
    row.append(description, button);
    return row;
  });
  javaResults.replaceChildren(...rows);
}
