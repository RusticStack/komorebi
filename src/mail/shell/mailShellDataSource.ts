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
  AccountAuthStatusRequest,
  AccountAuthStatusView,
  AccountConnectionTestResponse,
  AccountReauthRequest,
  AccountRemovalRequest,
  AccountRemovalResponse,
  AccountSetupRequest,
  AccountSetupResponse,
} from "./mailShellState";

const LOAD_MAIL_SHELL_COMMAND = "load_mail_shell";
const SEARCH_MAIL_COMMAND = "search_mail";
const AUTOSAVE_COMPOSER_DRAFT_COMMAND = "autosave_composer_draft";
const DISCARD_COMPOSER_DRAFT_COMMAND = "discard_composer_draft";
const STAGE_COMPOSER_ATTACHMENT_COMMAND = "stage_composer_attachment";
const SEND_COMPOSER_EMAIL_COMMAND = "send_composer_email";
const LIST_RECENT_RECIPIENTS_COMMAND = "list_recent_recipients";
const CREATE_ACCOUNT_COMMAND = "create_account";
const REAUTH_ACCOUNT_COMMAND = "reauth_account";
const REMOVE_ACCOUNT_COMMAND = "remove_account";
const TEST_ACCOUNT_CONNECTION_COMMAND = "test_account_connection";
const GET_ACCOUNT_AUTH_STATUS_COMMAND = "get_account_auth_status";

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

export async function createAccountFromTauri(
  request: AccountSetupRequest,
): Promise<AccountSetupResponse> {
  return invoke<AccountSetupResponse>(CREATE_ACCOUNT_COMMAND, { request });
}

export async function reauthAccountFromTauri(
  request: AccountReauthRequest,
): Promise<AccountSetupResponse> {
  return invoke<AccountSetupResponse>(REAUTH_ACCOUNT_COMMAND, { request });
}

export async function removeAccountFromTauri(
  request: AccountRemovalRequest,
): Promise<AccountRemovalResponse> {
  return invoke<AccountRemovalResponse>(REMOVE_ACCOUNT_COMMAND, { request });
}

export async function testAccountConnectionFromTauri(
  request: AccountSetupRequest,
): Promise<AccountConnectionTestResponse> {
  return invoke<AccountConnectionTestResponse>(TEST_ACCOUNT_CONNECTION_COMMAND, {
    request,
  });
}

export async function getAccountAuthStatusFromTauri(
  request: AccountAuthStatusRequest,
): Promise<AccountAuthStatusView> {
  return invoke<AccountAuthStatusView>(GET_ACCOUNT_AUTH_STATUS_COMMAND, {
    request,
  });
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) {
    throw new DOMException("Mail search was cancelled.", "AbortError");
  }
}
