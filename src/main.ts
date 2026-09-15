import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";

interface LauncherConfig {
  java_path: string;
  username: string;
  memory_mb: number;
  logs_enabled: boolean;
}

interface GameSetup {
  game_dir: string;
  version_id: string;
}

interface JavaInfo {
  version: string;
  is_64bit: boolean;
  raw: string;
}

interface LaunchHandle {
  pid: number;
  version_id: string;
}

const el = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

const javaPathInput = el<HTMLInputElement>("java-path");
const javaInfo = el<HTMLElement>("java-info");
const usernameInput = el<HTMLInputElement>("username");
const memRange = el<HTMLInputElement>("mem-range");
const memNum = el<HTMLInputElement>("mem-num");
const memLabel = el<HTMLElement>("mem-label");
const logsEnabled = el<HTMLInputElement>("logs-enabled");
const avatar = el<HTMLElement>("avatar");
const playerName = el<HTMLElement>("player-name");
const detectHint = el<HTMLElement>("detect-hint");
const statusEl = el<HTMLElement>("status");
const launchBtn = el<HTMLButtonElement>("launch");
const consoleEl = el<HTMLElement>("console");
const exportBtn = el<HTMLButtonElement>("export-log");

const MAX_LOG_LINES = 2000;

let running = false;
let saveTimer: number | undefined;
let stickToBottom = true;

function currentConfig(): LauncherConfig {
  return {
    java_path: javaPathInput.value.trim(),
    username: usernameInput.value.trim(),
    memory_mb: clampMemory(memNum.valueAsNumber),
    logs_enabled: logsEnabled.checked,
  };
}

function clampMemory(v: number): number {
  if (!Number.isFinite(v) || v <= 0) return 2048;
  return Math.min(65536, Math.max(512, Math.floor(v)));
}

function setStatus(text: string, cls: "idle" | "running" | "error"): void {
  statusEl.textContent = text;
  statusEl.className = `status ${cls}`;
}

function setJavaInfo(text: string, bad: boolean): void {
  javaInfo.textContent = text;
  javaInfo.classList.toggle("bad", bad);
}

function gotoPage(page: string): void {
  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.page === page);
  });
  document.querySelectorAll<HTMLElement>(".page").forEach((p) => {
    p.classList.toggle("active", p.id === `page-${page}`);
  });
}

function syncPlayer(): void {
  const name = usernameInput.value.trim() || "Player";
  playerName.textContent = name;
  avatar.textContent = (name[0] ?? "P").toUpperCase();
}

function scheduleSave(): void {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    invoke("save_config", { config: currentConfig() }).catch((e) =>
      console.error("save_config failed:", e),
    );
  }, 400);
}

function appendLog(line: string): void {
  consoleEl.appendChild(document.createTextNode(line + "\n"));
  while (consoleEl.childNodes.length > MAX_LOG_LINES) {
    consoleEl.removeChild(consoleEl.firstChild!);
  }
  if (stickToBottom) {
    consoleEl.scrollTop = consoleEl.scrollHeight;
  }
}

async function refreshDetect(): Promise<void> {
  try {
    const s = await invoke<GameSetup>("detect_setup");
    detectHint.textContent = `已识别 ${s.version_id}`;
    detectHint.title = s.game_dir;
    detectHint.classList.remove("bad");
  } catch (e) {
    detectHint.textContent = String(e);
    detectHint.title = "";
    detectHint.classList.add("bad");
  }
}

async function browseJava(): Promise<void> {
  const picked = await open({
    multiple: false,
    title: "选择 Java 可执行文件",
    filters: [{ name: "Java 可执行文件", extensions: ["exe"] }],
  });
  if (typeof picked === "string" && picked) {
    javaPathInput.value = picked;
    setJavaInfo("", false);
    scheduleSave();
  }
}

async function checkJava(): Promise<void> {
  const path = javaPathInput.value.trim();
  if (!path) {
    setJavaInfo("请先填写 Java 路径", true);
    return;
  }
  setJavaInfo("检测中…", false);
  try {
    const info = await invoke<JavaInfo>("validate_java", { javaPath: path });
    let text = `Java ${info.version}（${info.is_64bit ? "64 位" : "32 位"}）`;
    const over32bit = !info.is_64bit && currentConfig().memory_mb > 1536;
    if (over32bit) text += " ⚠ 32 位 Java 分配超过 1536 MB 可能无法启动";
    setJavaInfo(text, over32bit);
  } catch (e) {
    setJavaInfo(`无法运行：${e}`, true);
  }
}

async function launch(): Promise<void> {
  if (running) return;
  const cfg = currentConfig();
  if (!cfg.username) return setStatus("请先在设置中填写玩家名", "error");
  if (!cfg.java_path) return setStatus("请先在设置中选择 Java 可执行文件", "error");

  try {
    const h = await invoke<LaunchHandle>("launch", { config: cfg });
    running = true;
    launchBtn.disabled = true;
    launchBtn.textContent = "运行中";
    setStatus(`运行中 · ${h.version_id} · PID ${h.pid}`, "running");
    appendLog(`[Nova] 已启动 ${h.version_id}，PID ${h.pid}`);
  } catch (e) {
    setStatus(String(e), "error");
    appendLog(`[Nova] 启动失败：${e}`);
  }
}

async function exportLog(): Promise<void> {
  const content = consoleEl.textContent ?? "";
  if (!content.trim()) {
    setStatus("日志为空，无可导出", "error");
    return;
  }
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
  const path = await save({
    title: "导出游戏日志",
    defaultPath: `nova-launcher-${stamp}.log`,
    filters: [{ name: "日志文件", extensions: ["log", "txt"] }],
  });
  if (!path) return;
  try {
    await invoke("write_log_file", { path, content });
    setStatus("日志已导出", "idle");
  } catch (e) {
    setStatus(String(e), "error");
  }
}

function setMemory(mb: number): void {
  memRange.value = String(clampMemory(mb));
  memNum.value = memRange.value;
  memLabel.textContent = `${memRange.value} MB`;
}

async function init(): Promise<void> {
  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((b) => {
    b.addEventListener("click", () => gotoPage(b.dataset.page!));
  });
  el<HTMLAnchorElement>("edit-profile").addEventListener("click", (e) => {
    e.preventDefault();
    gotoPage("settings");
  });

  usernameInput.addEventListener("input", () => {
    syncPlayer();
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
  logsEnabled.addEventListener("change", scheduleSave);
  el<HTMLButtonElement>("java-browse").addEventListener("click", () => void browseJava());
  el<HTMLButtonElement>("java-check").addEventListener("click", () => void checkJava());
  launchBtn.addEventListener("click", () => void launch());
  exportBtn.addEventListener("click", () => void exportLog());
  consoleEl.addEventListener("scroll", () => {
    stickToBottom = consoleEl.scrollHeight - consoleEl.scrollTop - consoleEl.clientHeight < 24;
  });

  try {
    await listen<string>("game-log", (event) => appendLog(event.payload));
  } catch (e) {
    console.warn("game-log listener unavailable:", e);
  }
  try {
    await listen<number | null>("game-exit", (event) => {
      running = false;
      launchBtn.disabled = false;
      launchBtn.textContent = "启 动";
      const code = event.payload;
      if (code === null) {
        setStatus("进程已被强制终止", "error");
        appendLog("[Nova] 进程被强制终止");
      } else if (code === 0) {
        setStatus("已退出", "idle");
        appendLog("[Nova] 进程正常退出");
      } else {
        setStatus(`异常退出 · 退出码 ${code}`, "error");
        appendLog(`[Nova] 进程异常退出，退出码 ${code}（详情见日志 / crash-reports）`);
      }
    });
  } catch (e) {
    console.warn("game-exit listener unavailable:", e);
  }

  try {
    const cfg = await invoke<LauncherConfig | null>("load_config");
    if (cfg) {
      javaPathInput.value = cfg.java_path ?? "";
      usernameInput.value = cfg.username || "Player";
      logsEnabled.checked = cfg.logs_enabled ?? true;
      setMemory(cfg.memory_mb ?? 2048);
    } else {
      usernameInput.value = "Player";
      setMemory(2048);
    }
  } catch (e) {
    console.error("load_config failed:", e);
    usernameInput.value = "Player";
    setMemory(2048);
  }
  syncPlayer();
  await refreshDetect();
}

window.addEventListener("DOMContentLoaded", () => void init());
