import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ComposerAttachment,
  ComposerForwardedAttachmentStageRequest,
  ComposerAttachmentStageRequest,
  ComposerDraftDiscardRequest,
  ComposerDraft,
  ComposerDraftSaveRequest,
  ComposerDraftSaveResponse,
  ComposerSendRequest,
  ComposerSendResponse,
  AttachmentCommandRequest,
  AttachmentDownloadResponse,
  AttachmentPreviewResponse,
  AttachmentSaveResponse,
  ExternalLinkOpenRequest,
  ExternalLinkOpenResponse,
  MailActionRequest,
  MailActionResponse,
  RemoteContentPreferenceRequest,
  RemoteContentPreferenceResponse,
  MailSearchRequest,
  MailSearchResponse,
  MailMessagePageRequest,
  MailMessagePageResponse,
  MailShellData,
  RecentRecipientSearchRequest,
  RecentRecipientSuggestion,
  AccountAuthStatusRequest,
  AccountAuthStatusView,
  AccountConnectionTestResponse,
  AccountSettingsUpdateRequest,
  AccountSettingsUpdateResponse,
  OAuthAccountReauthRequest,
  OAuthAccountSetupRequest,
  AccountReauthRequest,
  AccountRemovalRequest,
  AccountRemovalResponse,
  AccountSetupRequest,
  AccountSetupResponse,
  UiPreferencesUpdateRequest,
  UiPreferencesView,
  DiagnosticsExportRequest,
  DiagnosticsExportResponse,
  DiagnosticsHealthView,
} from "./mailShellState";

export const SYNC_STATUS_EVENT_NAME = "komorebi://sync-status";

export type SyncStatusEventPayload = {
  accountId: string;
  status: {
    state: string;
    updatedAt: number;
    lastSuccessfulSyncAt?: number | null;
    message?: string | null;
    action?: string | null;
    code?: string | null;
  };
};

export type SyncStatusEventHandler = (
  payload: SyncStatusEventPayload,
) => void;

const LOAD_MAIL_SHELL_COMMAND = "load_mail_shell";
const LOAD_MESSAGE_PAGE_COMMAND = "load_message_page";
const SEARCH_MAIL_COMMAND = "search_mail";
const ALLOW_REMOTE_CONTENT_COMMAND = "allow_remote_content";
const GET_UI_PREFERENCES_COMMAND = "get_ui_preferences";
const UPDATE_UI_PREFERENCES_COMMAND = "update_ui_preferences";
const PERFORM_MAIL_ACTION_COMMAND = "perform_mail_action";
const OPEN_EXTERNAL_LINK_COMMAND = "open_external_link";
const PREVIEW_ATTACHMENT_COMMAND = "preview_attachment";
const SAVE_ATTACHMENT_COMMAND = "save_attachment";
const DOWNLOAD_ATTACHMENT_COMMAND = "download_attachment";
const AUTOSAVE_COMPOSER_DRAFT_COMMAND = "autosave_composer_draft";
const DISCARD_COMPOSER_DRAFT_COMMAND = "discard_composer_draft";
const STAGE_COMPOSER_ATTACHMENT_COMMAND = "stage_composer_attachment";
const STAGE_FORWARDED_ATTACHMENTS_COMMAND = "stage_forwarded_attachments";
const SEND_COMPOSER_EMAIL_COMMAND = "send_composer_email";
const LIST_RECENT_RECIPIENTS_COMMAND = "list_recent_recipients";
const LIST_COMPOSER_DRAFTS_COMMAND = "list_composer_drafts";
const CREATE_ACCOUNT_COMMAND = "create_account";
const REAUTH_ACCOUNT_COMMAND = "reauth_account";
const REMOVE_ACCOUNT_COMMAND = "remove_account";
const UPDATE_ACCOUNT_SETTINGS_COMMAND = "update_account_settings";
const TEST_ACCOUNT_CONNECTION_COMMAND = "test_account_connection";
const GET_ACCOUNT_AUTH_STATUS_COMMAND = "get_account_auth_status";
const START_OAUTH_ACCOUNT_SETUP_COMMAND = "start_oauth_account_setup";
const START_OAUTH_ACCOUNT_REAUTH_COMMAND = "start_oauth_account_reauth";
const EXPORT_DIAGNOSTICS_BUNDLE_COMMAND = "export_diagnostics_bundle";
const GET_DIAGNOSTICS_HEALTH_COMMAND = "get_diagnostics_health";

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

export async function loadMessagePageFromTauri(
  request: MailMessagePageRequest,
  signal: AbortSignal,
): Promise<MailMessagePageResponse> {
  throwIfAborted(signal);
  const response = await invoke<MailMessagePageResponse>(
    LOAD_MESSAGE_PAGE_COMMAND,
    { request },
  );
  throwIfAborted(signal);

  return response;
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

export async function allowRemoteContentFromTauri(
  request: RemoteContentPreferenceRequest,
): Promise<RemoteContentPreferenceResponse> {
  return invoke<RemoteContentPreferenceResponse>(ALLOW_REMOTE_CONTENT_COMMAND, {
    request,
  });
}

export async function getUiPreferencesFromTauri(): Promise<UiPreferencesView> {
  return invoke<UiPreferencesView>(GET_UI_PREFERENCES_COMMAND);
}

export async function updateUiPreferencesFromTauri(
  request: UiPreferencesUpdateRequest,
): Promise<UiPreferencesView> {
  return invoke<UiPreferencesView>(UPDATE_UI_PREFERENCES_COMMAND, {
    request,
  });
}

export async function openExternalLinkFromTauri(
  request: ExternalLinkOpenRequest,
): Promise<ExternalLinkOpenResponse> {
  return invoke<ExternalLinkOpenResponse>(OPEN_EXTERNAL_LINK_COMMAND, {
    request,
  });
}

export async function performMailActionFromTauri(
  request: MailActionRequest,
): Promise<MailActionResponse> {
  return invoke<MailActionResponse>(PERFORM_MAIL_ACTION_COMMAND, {
    request,
  });
}

export async function previewAttachmentFromTauri(
  request: AttachmentCommandRequest,
): Promise<AttachmentPreviewResponse> {
  return invoke<AttachmentPreviewResponse>(PREVIEW_ATTACHMENT_COMMAND, {
    request,
  });
}

export async function saveAttachmentFromTauri(
  request: AttachmentCommandRequest,
): Promise<AttachmentSaveResponse> {
  return invoke<AttachmentSaveResponse>(SAVE_ATTACHMENT_COMMAND, {
    request,
  });
}

export async function downloadAttachmentFromTauri(
  request: AttachmentCommandRequest,
): Promise<AttachmentDownloadResponse> {
  return invoke<AttachmentDownloadResponse>(DOWNLOAD_ATTACHMENT_COMMAND, {
    request,
  });
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

export async function stageForwardedAttachmentsFromTauri(
  request: ComposerForwardedAttachmentStageRequest,
): Promise<ComposerAttachment[]> {
  return invoke<ComposerAttachment[]>(STAGE_FORWARDED_ATTACHMENTS_COMMAND, {
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

export async function listComposerDraftsFromTauri(): Promise<ComposerDraft[]> {
  return invoke<ComposerDraft[]>(LIST_COMPOSER_DRAFTS_COMMAND);
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

export async function startOAuthAccountSetupFromTauri(
  request: OAuthAccountSetupRequest,
): Promise<AccountSetupResponse> {
  return invoke<AccountSetupResponse>(START_OAUTH_ACCOUNT_SETUP_COMMAND, {
    request,
  });
}

export async function startOAuthAccountReauthFromTauri(
  request: OAuthAccountReauthRequest,
): Promise<AccountSetupResponse> {
  return invoke<AccountSetupResponse>(START_OAUTH_ACCOUNT_REAUTH_COMMAND, {
    request,
  });
}

export async function removeAccountFromTauri(
  request: AccountRemovalRequest,
): Promise<AccountRemovalResponse> {
  return invoke<AccountRemovalResponse>(REMOVE_ACCOUNT_COMMAND, { request });
}

export async function updateAccountSettingsFromTauri(
  request: AccountSettingsUpdateRequest,
): Promise<AccountSettingsUpdateResponse> {
  return invoke<AccountSettingsUpdateResponse>(UPDATE_ACCOUNT_SETTINGS_COMMAND, {
    request,
  });
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

export async function exportDiagnosticsBundleFromTauri(
  request: DiagnosticsExportRequest,
): Promise<DiagnosticsExportResponse> {
  return invoke<DiagnosticsExportResponse>(EXPORT_DIAGNOSTICS_BUNDLE_COMMAND, {
    request,
  });
}

export async function getDiagnosticsHealthFromTauri(): Promise<DiagnosticsHealthView> {
  return invoke<DiagnosticsHealthView>(GET_DIAGNOSTICS_HEALTH_COMMAND);
}

export async function subscribeToSyncStatusFromTauri(
  handler: SyncStatusEventHandler,
): Promise<UnlistenFn> {
  return listen<SyncStatusEventPayload>(SYNC_STATUS_EVENT_NAME, (event) => {
    handler(event.payload);
  });
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) {
    throw new DOMException("Mail search was cancelled.", "AbortError");
  }
}
