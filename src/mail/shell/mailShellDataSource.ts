import { invoke } from "@tauri-apps/api/core";
import type { MailShellData } from "./mailShellState";

const LOAD_MAIL_SHELL_COMMAND = "load_mail_shell";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function loadMailShellDataFromTauri(): Promise<MailShellData> {
  return invoke<MailShellData>(LOAD_MAIL_SHELL_COMMAND);
}
