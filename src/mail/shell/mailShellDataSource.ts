import { invoke } from "@tauri-apps/api/core";
import type {
  MailSearchRequest,
  MailSearchResponse,
  MailShellData,
} from "./mailShellState";

const LOAD_MAIL_SHELL_COMMAND = "load_mail_shell";
const SEARCH_MAIL_COMMAND = "search_mail";

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

export async function searchMailFromTauri(
  request: MailSearchRequest,
  signal: AbortSignal,
): Promise<MailSearchResponse> {
  throwIfAborted(signal);
  const response = await invoke<MailSearchResponse>(SEARCH_MAIL_COMMAND, {
    request,
  });
  throwIfAborted(signal);

  return response;
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) {
    throw new DOMException("Mail search was cancelled.", "AbortError");
  }
}
