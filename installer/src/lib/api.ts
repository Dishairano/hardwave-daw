import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export interface LatestVersion {
  version: string;
  notes: string;
  pub_date?: string;
}

export interface InstallProgress {
  phase:
    | "downloading"
    | "extracting"
    | "shortcuts"
    | "registering"
    | "done";
  percent: number;
  message: string;
}

export interface InstallOptions {
  install_dir: string;
  create_desktop_shortcut: boolean;
  create_start_menu_shortcut: boolean;
  launch_after: boolean;
}

export async function fetchLatestVersion(): Promise<LatestVersion> {
  return invoke<LatestVersion>("fetch_latest_version");
}

export async function defaultInstallDir(): Promise<string> {
  return invoke<string>("default_install_dir");
}

export async function startInstall(options: InstallOptions): Promise<string> {
  return invoke<string>("start_install", { options });
}

export async function cancelInstall(): Promise<void> {
  return invoke("cancel_install");
}

export async function launchInstalled(exePath: string): Promise<void> {
  return invoke("launch_installed", { exePath });
}

export async function quit(): Promise<void> {
  return invoke("quit");
}

export async function onProgress(
  handler: (p: InstallProgress) => void
): Promise<UnlistenFn> {
  return listen<InstallProgress>("install://progress", (e) => handler(e.payload));
}

// ─── Removal (the same program, started with --uninstall) ────────────────────

export type LaunchMode = "install" | "uninstall";

export interface UninstallProgress {
  /** Index of the step that just started, in uninstallSteps(). */
  step: number;
  message: string;
}

export async function launchMode(): Promise<LaunchMode> {
  return invoke<LaunchMode>("launch_mode");
}

export async function uninstallSteps(): Promise<string[]> {
  return invoke<string[]>("uninstall_steps");
}

/** Rejects with "daw-running", "not-installed" or a sentence. */
export async function startUninstall(removeSettings: boolean): Promise<void> {
  return invoke("start_uninstall", { removeSettings });
}

export async function onUninstallProgress(
  handler: (p: UninstallProgress) => void
): Promise<UnlistenFn> {
  return listen<UninstallProgress>("uninstall://progress", (e) => handler(e.payload));
}
