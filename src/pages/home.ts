/** Home page: detected setup, launch button, and the game's lifecycle events. */

import * as backend from "../api/backend";
import { currentConfig } from "../config";
import { launchBtn } from "../ui/dom";
import { setDetectHint, setStatus } from "../ui/status";
import { appendLog } from "./logs";

let running = false;

export async function initHomePage(): Promise<void> {
  launchBtn.addEventListener("click", () => void launch());

  try {
    await backend.onGameLog(appendLog);
  } catch (e) {
    console.warn("game-log listener unavailable:", e);
  }
  try {
    await backend.onGameExit(onGameExit);
  } catch (e) {
    console.warn("game-exit listener unavailable:", e);
  }
}

/** Re-reads the auto-detected `.minecraft` + version shown under the button. */
export async function refreshDetect(): Promise<void> {
  try {
    const setup = await backend.detectSetup();
    setDetectHint(`已识别 ${setup.version_id}`, setup.game_dir);
  } catch (e) {
    setDetectHint(String(e), "", true);
  }
}

function resetLaunchButton(): void {
  running = false;
  launchBtn.disabled = false;
  launchBtn.textContent = "启 动";
}

function onGameExit(code: number | null): void {
  resetLaunchButton();
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
}

async function launch(): Promise<void> {
  if (running) return;
  const cfg = currentConfig();
  if (!cfg.username) {
    setStatus("请先在设置中填写玩家名", "error");
    return;
  }
  if (!cfg.java_path) {
    setStatus("请先在设置中选择 Java 可执行文件", "error");
    return;
  }

  try {
    const handle = await backend.launchGame(cfg);
    running = true;
    launchBtn.disabled = true;
    launchBtn.textContent = "运行中";
    setStatus(`运行中 · ${handle.version_id} · PID ${handle.pid}`, "running");
    appendLog(`[Nova] 已启动 ${handle.version_id}，PID ${handle.pid}`);
  } catch (e) {
    setStatus(String(e), "error");
    appendLog(`[Nova] 启动失败：${e}`);
  }
}
