import { invoke } from "@tauri-apps/api/core";
import type {
  ComposerAttachment,
  ComposerAttachmentStageRequest,
  ComposerDraftDiscardRequest,
  ComposerDraftSaveRequest,
  ComposerDraftSaveResponse,
  ComposerSendRequest,
  ComposerSendResponse,
  MailSearchRequest,
  MailSearchResponse,
  MailShellData,
  RecentRecipientSearchRequest,
  RecentRecipientSuggestion,
} from "./mailShellState";

const LOAD_MAIL_SHELL_COMMAND = "load_mail_shell";
const SEARCH_MAIL_COMMAND = "search_mail";
const AUTOSAVE_COMPOSER_DRAFT_COMMAND = "autosave_composer_draft";
const DISCARD_COMPOSER_DRAFT_COMMAND = "discard_composer_draft";
const STAGE_COMPOSER_ATTACHMENT_COMMAND = "stage_composer_attachment";
const SEND_COMPOSER_EMAIL_COMMAND = "send_composer_email";
const LIST_RECENT_RECIPIENTS_COMMAND = "list_recent_recipients";

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

export async function autosaveComposerDraftFromTauri(
  request: ComposerDraftSaveRequest,
): Promise<ComposerDraftSaveResponse> {
  return invoke<ComposerDraftSaveResponse>(AUTOSAVE_COMPOSER_DRAFT_COMMAND, {
    request,
  });
}

export async function discardComposerDraftFromTauri(
  request: ComposerDraftDiscardRequest,
): Promise<ComposerDraftSaveResponse> {
  return invoke<ComposerDraftSaveResponse>(DISCARD_COMPOSER_DRAFT_COMMAND, {
    request,
  });
}

export async function stageComposerAttachmentFromTauri(
  request: ComposerAttachmentStageRequest,
): Promise<ComposerAttachment> {
  return invoke<ComposerAttachment>(STAGE_COMPOSER_ATTACHMENT_COMMAND, {
    request,
  });
}

export async function sendComposerMailFromTauri(
  request: ComposerSendRequest,
): Promise<ComposerSendResponse> {
  return invoke<ComposerSendResponse>(SEND_COMPOSER_EMAIL_COMMAND, {
    request,
  });
}

export async function listRecentRecipientsFromTauri(
  request: RecentRecipientSearchRequest,
  signal: AbortSignal,
): Promise<RecentRecipientSuggestion[]> {
  throwIfAborted(signal);
  const response = await invoke<RecentRecipientSuggestion[]>(
    LIST_RECENT_RECIPIENTS_COMMAND,
    {
      request,
    },
  );
  throwIfAborted(signal);

  return response;
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) {
    throw new DOMException("Mail search was cancelled.", "AbortError");
  }
}
