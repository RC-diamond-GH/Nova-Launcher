/** Payload shapes of the Rust commands in `src-tauri/src/`. */

export interface LauncherConfig {
  java_path: string;
  username: string;
  memory_mb: number;
  logs_enabled: boolean;
}

export interface GameSetup {
  game_dir: string;
  version_id: string;
}

export interface JavaInfo {
  version: string;
  is_64bit: boolean;
  raw: string;
}

export interface LaunchHandle {
  pid: number;
  version_id: string;
}
