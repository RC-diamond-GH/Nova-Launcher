/**
 * The only module that talks to the Rust side: every `invoke` / `listen` /
 * native dialog call lives here, so pages stay DOM + orchestration.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";

import type { GameSetup, JavaDiscovery, JavaInfo, LauncherConfig, LaunchHandle } from "./types";

export function loadConfig(): Promise<LauncherConfig | null> {
  return invoke<LauncherConfig | null>("load_config");
}

export function saveConfig(config: LauncherConfig): Promise<void> {
  return invoke("save_config", { config });
}

export function detectSetup(): Promise<GameSetup> {
  return invoke<GameSetup>("detect_setup");
}

export function validateJava(javaPath: string): Promise<JavaInfo> {
  return invoke<JavaInfo>("validate_java", { javaPath });
}

export function discoverJava(): Promise<JavaDiscovery> {
  return invoke<JavaDiscovery>("discover_java");
}

export function writeLogFile(path: string, content: string): Promise<void> {
  return invoke("write_log_file", { path, content });
}

export function launchGame(config: LauncherConfig): Promise<LaunchHandle> {
  return invoke<LaunchHandle>("launch", { config });
}

export function onGameLog(handler: (line: string) => void): Promise<UnlistenFn> {
  return listen<string>("game-log", (event) => handler(event.payload));
}

export function onGameExit(handler: (code: number | null) => void): Promise<UnlistenFn> {
  return listen<number | null>("game-exit", (event) => handler(event.payload));
}

/** Native file picker for the java executable; `null` when cancelled. */
export async function pickJavaExecutable(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    title: "选择 Java 可执行文件",
    filters: [{ name: "Java 可执行文件", extensions: ["exe", "*"] }],
  });
  return typeof picked === "string" && picked ? picked : null;
}

/** Native save dialog for an exported log; `null` when cancelled. */
export async function pickLogPath(defaultPath: string): Promise<string | null> {
  const path = await save({
    title: "导出游戏日志",
    defaultPath,
    filters: [{ name: "日志文件", extensions: ["log", "txt"] }],
  });
  return path ?? null;
}
