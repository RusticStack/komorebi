import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useLocaleFormatters } from "../../i18n";
import { SafeEmailBody } from "../rendering/SafeEmailBody";
import {
  hasRemoteContentReferences,
  type SafeEmailLink,
} from "../rendering/emailHtml";
import { messageSenderLabel } from "./mailDisplay";
import type {
  AttachmentPreviewResponse,
  MailActionKind,
  MailActionRequest,
  MailActionResponse,
  MailActionUndo,
  MailAttachment,
  MailMessage,
  MailShellState,
  RemoteContentPreferenceRequest,
  RemoteContentPreferenceResponse,
  RemoteContentPreferenceScope,
} from "./mailShellState";

type RemoteContentPreferenceFunction = (
  request: RemoteContentPreferenceRequest,
) => Promise<RemoteContentPreferenceResponse>;
type AttachmentPreviewFunction = unknown;
type AttachmentSaveFunction = unknown;
type AttachmentDownloadFunction = unknown;
type MailActionFunction = (request: MailActionRequest) => Promise<MailActionResponse>;
type ComposerRouteKind = "reply" | "reply-all" | "forward";

type AttachmentActionState = {
  attachmentId: string | null;
  status: "idle" | "loading" | "saved" | "downloaded" | "error";
  message: string | null;
};
type RemoteContentActionState = {
  messageId: string | null;
  status: "idle" | "loading" | "saved" | "error";
  message: string | null;
};
type ExternalLinkActionState = {
  linkId: string | null;
  status: "idle" | "opening" | "opened" | "error";
  message: string | null;
};
type ComposerRouteActionState = {
  status: "idle" | "loading" | "error";
  message: string | null;
};
type MailActionUiState = {
  status: "idle" | "running" | "succeeded" | "error";
  message: string | null;
  undo: MailActionUndo | null;
  retryRequest: MailActionRequest | null;
};
type MailActionConfirmation = {
  request: MailActionRequest;
  action: "delete-to-trash" | "expunge-deleted";
} | null;

const ATTACHMENT_PREVIEW_MAX_BYTES = 5 * 1024 * 1024;
const PREVIEWABLE_ATTACHMENT_TYPES = new Set([
  "image/gif",
  "image/jpeg",
  "image/png",
  "image/webp",
  "text/plain",
]);

function mailActionRequestFor(
  message: MailMessage,
  action: MailActionKind,
  confirmed: boolean,
): MailActionRequest {
  return {
    accountId: message.accountId,
    action,
    confirmed,
    folderId: message.folderId,
    messageId: action.kind === "expunge-deleted" ? null : message.id,
    userRequested: true,
  };
}

function bytesToDataUrl(contentType: string, bytes: number[]) {
  let binary = "";
  const chunkSize = 8192;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.slice(index, index + chunkSize));
  }

  return `data:${contentType};base64,${btoa(binary)}`;
}

function canPreviewAttachmentMetadata(attachment: MailAttachment) {
  return (
    PREVIEWABLE_ATTACHMENT_TYPES.has(attachment.contentType.toLocaleLowerCase()) &&
    attachment.sizeBytes <= ATTACHMENT_PREVIEW_MAX_BYTES
  );
}

export function ReaderPane({
  state,
  message,
  hasVisibleMessages,
  canCompose,
  canForwardAttachments,
  onRetry,
  allowRemoteContent,
  previewAttachment,
  saveAttachment,
  downloadAttachment,
  attachmentPreview,
  attachmentAction,
  remoteContentAction,
  externalLinkAction,
  composerRouteAction,
  mailAction,
  mailActionConfirmation,
  performMailAction,
  onAllowRemoteContent,
  onOpenExternalLink,
  onRouteComposer,
  onRequestMailAction,
  onConfirmMailAction,
  onCancelMailAction,
  onRetryMailAction,
  onUndoMailAction,
  onPreviewAttachment,
  onSaveAttachment,
  onDownloadAttachment,
  onCloseAttachmentPreview,
}: {
  state: MailShellState;
  message: MailMessage | null;
  hasVisibleMessages: boolean;
  canCompose: boolean;
  canForwardAttachments: boolean;
  onRetry?: () => void;
  allowRemoteContent?: RemoteContentPreferenceFunction;
  previewAttachment?: AttachmentPreviewFunction;
  saveAttachment?: AttachmentSaveFunction;
  downloadAttachment?: AttachmentDownloadFunction;
  attachmentPreview: AttachmentPreviewResponse | null;
  attachmentAction: AttachmentActionState;
  remoteContentAction: RemoteContentActionState;
  externalLinkAction: ExternalLinkActionState;
  composerRouteAction: ComposerRouteActionState;
  mailAction: MailActionUiState;
  mailActionConfirmation: MailActionConfirmation;
  performMailAction?: MailActionFunction;
  onAllowRemoteContent: (
    message: MailMessage,
    scope: RemoteContentPreferenceScope,
  ) => Promise<void>;
  onOpenExternalLink: (link: SafeEmailLink) => Promise<void>;
  onRouteComposer: (
    message: MailMessage,
    route: ComposerRouteKind,
    includeAttachments: boolean,
    restoreFocus: HTMLElement | null,
  ) => void;
  onRequestMailAction: (request: MailActionRequest) => void;
  onConfirmMailAction: () => void;
  onCancelMailAction: () => void;
  onRetryMailAction: () => void;
  onUndoMailAction: () => void;
  onPreviewAttachment: (message: MailMessage, attachment: MailAttachment) => void;
  onSaveAttachment: (message: MailMessage, attachment: MailAttachment) => void;
  onDownloadAttachment: (message: MailMessage, attachment: MailAttachment) => void;
  onCloseAttachmentPreview: () => void;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();
  const sender = message ? messageSenderLabel(message, t) : "";
  const [remoteContentPanelOpen, setRemoteContentPanelOpen] = useState(false);
  const messageHasRemoteContent = hasRemoteContentReferences(message?.bodyHtml);
  const remoteContentAllowed = Boolean(message?.remoteContentAllowed);
  const remoteContentAllowedScope = message?.remoteContentAllowedScope ?? null;
  const remoteContentBusy =
    Boolean(message) &&
    remoteContentAction.messageId === message?.id &&
    remoteContentAction.status === "loading";
  const remoteContentStatusMessage =
    message && remoteContentAction.messageId === message.id
      ? remoteContentAction.message
      : null;
  const externalLinkStatusMessage = externalLinkAction.message;
  const currentFolder = message
    ? state.data.folders.find(
        (folder) =>
          folder.accountId === message.accountId && folder.id === message.folderId,
      ) ?? null
    : null;
  const accountFolders = message
    ? state.data.folders.filter((folder) => folder.accountId === message.accountId)
    : [];
  const archiveFolder = accountFolders.find((folder) => folder.kind === "archive");
  const trashFolder = accountFolders.find((folder) => folder.kind === "trash");
  const moveTargetFolders = accountFolders.filter(
    (folder) => folder.id !== message?.folderId && folder.kind !== "all",
  );
  const mailActionBusy = mailAction.status === "running";
  const mailActionAvailable = Boolean(performMailAction);
  const composerRouteBusy = composerRouteAction.status === "loading";

  if (state.loadState.messages === "loading") {
    return (
      <>
        <section className="reader-empty" aria-label={t("mail.reader.loading.label")}>
          <p className="eyebrow">{t("mail.reader.eyebrow")}</p>
          <h2>{t("mail.reader.loading.heading")}</h2>
          <p>{t("mail.reader.loading.body")}</p>
        </section>
        <ReaderStatus text={t("mail.reader.loading.status")} />
      </>
    );
  }

  if (state.loadState.messages === "error") {
    return (
      <>
        <section className="reader-empty" aria-label={t("mail.reader.error.label")}>
          <p className="eyebrow">{t("mail.reader.eyebrow")}</p>
          <h2>{t("mail.reader.error.heading")}</h2>
          <p>{t("mail.reader.error.body")}</p>
          <button type="button" onClick={onRetry} disabled={!onRetry}>
            {t("common.actions.retry")}
          </button>
        </section>
        <ReaderStatus text={t("common.status.errorState")} />
      </>
    );
  }

  if (!message) {
    return (
      <>
        <section className="reader-empty" aria-label={t("mail.reader.empty.label")}>
          <p className="eyebrow">{t("mail.reader.eyebrow")}</p>
          <h2>{t("mail.reader.empty.heading")}</h2>
          <p>
            {hasVisibleMessages
              ? t("mail.reader.empty.bodyWithMessages")
              : t("mail.reader.empty.body")}
          </p>
        </section>
        <ReaderStatus
          text={
            state.data.accounts.length === 0
              ? t("mail.reader.empty.accountSetupRequired")
              : t("mail.reader.remoteBlocked")
          }
        />
      </>
    );
  }

  return (
    <>
      <header className="reader-header">
        <p className="eyebrow">{t("mail.reader.selectedEyebrow")}</p>
        <h2>{message.subject || t("mail.messages.noSubject")}</h2>
        <div className="sender-line">
          <span>{sender}</span>
          <time
            dateTime={message.receivedAt}
            title={localeFormat.dateTime(message.receivedAt)}
          >
            {localeFormat.relativeDateTime(message.receivedAt)}
          </time>
        </div>
      </header>

      <div className="reader-actions" aria-label={t("mail.reader.actions.label")}>
        <button
          type="button"
          disabled={!canCompose || composerRouteBusy}
          title={
            canCompose
              ? t("mail.reader.actions.replyTitle")
              : t("mail.reader.actions.replyUnavailableTitle")
          }
          aria-label={
            canCompose
              ? t("mail.reader.actions.reply")
              : t("mail.reader.actions.replyUnavailable")
          }
          onClick={(event) =>
            onRouteComposer(message, "reply", false, event.currentTarget)
          }
        >
          {t("mail.reader.actions.reply")}
        </button>
        <button
          type="button"
          disabled={!canCompose || composerRouteBusy}
          title={
            canCompose
              ? t("mail.reader.actions.replyAllTitle")
              : t("mail.reader.actions.replyAllUnavailableTitle")
          }
          aria-label={
            canCompose
              ? t("mail.reader.actions.replyAll")
              : t("mail.reader.actions.replyAllUnavailable")
          }
          onClick={(event) =>
            onRouteComposer(message, "reply-all", false, event.currentTarget)
          }
        >
          {t("mail.reader.actions.replyAll")}
        </button>
        <button
          type="button"
          disabled={!canCompose || composerRouteBusy}
          title={
            canCompose
              ? t("mail.reader.actions.forwardTitle")
              : t("mail.reader.actions.forwardUnavailableTitle")
          }
          aria-label={
            canCompose
              ? t("mail.reader.actions.forward")
              : t("mail.reader.actions.forwardUnavailable")
          }
          onClick={(event) =>
            onRouteComposer(message, "forward", false, event.currentTarget)
          }
        >
          {t("mail.reader.actions.forward")}
        </button>
        {message.attachments.length > 0 ? (
          <button
            type="button"
            disabled={!canCompose || !canForwardAttachments || composerRouteBusy}
            title={
              canCompose && canForwardAttachments
                ? t("mail.reader.actions.forwardWithAttachmentsTitle")
                : t("mail.reader.actions.forwardWithAttachmentsUnavailableTitle")
            }
            aria-label={
              canCompose && canForwardAttachments
                ? t("mail.reader.actions.forwardWithAttachments")
                : t("mail.reader.actions.forwardWithAttachmentsUnavailable")
            }
            onClick={(event) =>
              onRouteComposer(message, "forward", true, event.currentTarget)
            }
          >
            {t("mail.reader.actions.forwardWithAttachments")}
          </button>
        ) : null}
        <button
          type="button"
          disabled={
            !mailActionAvailable ||
            !archiveFolder ||
            archiveFolder.id === message.folderId ||
            mailActionBusy
          }
          title={
            mailActionAvailable && archiveFolder
              ? t("mail.reader.actions.archiveTitle")
              : t("mail.reader.actions.archiveUnavailableTitle")
          }
          aria-label={
            mailActionAvailable && archiveFolder
              ? t("mail.reader.actions.archive")
              : t("mail.reader.actions.archiveUnavailable")
          }
          onClick={() =>
            onRequestMailAction(
              mailActionRequestFor(message, { kind: "archive" }, false),
            )
          }
        >
          {t("mail.reader.actions.archive")}
        </button>
        <button
          type="button"
          disabled={!mailActionAvailable || mailActionBusy}
          title={
            mailActionAvailable
              ? t("mail.reader.actions.readStateTitle")
              : t("mail.reader.actions.readStateUnavailableTitle")
          }
          aria-label={
            mailActionAvailable
              ? message.unread
                ? t("mail.reader.actions.markRead")
                : t("mail.reader.actions.markUnread")
              : message.unread
                ? t("mail.reader.actions.markReadUnavailable")
                : t("mail.reader.actions.markUnreadUnavailable")
          }
          onClick={() =>
            onRequestMailAction(
              mailActionRequestFor(
                message,
                { kind: "mark-read", read: message.unread },
                false,
              ),
            )
          }
        >
          {message.unread
            ? t("mail.reader.actions.markRead")
            : t("mail.reader.actions.markUnread")}
        </button>
        <label className="reader-actions__move">
          <span>{t("mail.reader.actions.moveLabel")}</span>
          <select
            aria-label={t("mail.reader.actions.moveLabel")}
            disabled={
              !mailActionAvailable ||
              mailActionBusy ||
              moveTargetFolders.length === 0
            }
            value=""
            onChange={(event) => {
              const targetFolderId = event.target.value;
              if (!targetFolderId) {
                return;
              }
              onRequestMailAction(
                mailActionRequestFor(
                  message,
                  { kind: "move", targetFolderId },
                  false,
                ),
              );
              event.currentTarget.value = "";
            }}
          >
            <option value="">{t("mail.reader.actions.movePlaceholder")}</option>
            {moveTargetFolders.map((folder) => (
              <option key={folder.id} value={folder.id}>
                {folder.name}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          disabled={
            !mailActionAvailable ||
            !trashFolder ||
            trashFolder.id === message.folderId ||
            mailActionBusy
          }
          title={
            mailActionAvailable && trashFolder
              ? t("mail.reader.actions.deleteTitle")
              : t("mail.reader.actions.deleteUnavailableTitle")
          }
          aria-label={
            mailActionAvailable && trashFolder
              ? t("mail.reader.actions.delete")
              : t("mail.reader.actions.deleteUnavailable")
          }
          onClick={() =>
            onRequestMailAction(
              mailActionRequestFor(message, { kind: "delete-to-trash" }, false),
            )
          }
        >
          {t("mail.reader.actions.delete")}
        </button>
        {currentFolder?.kind === "trash" ? (
          <button
            type="button"
            disabled={!mailActionAvailable || mailActionBusy}
            title={
              mailActionAvailable
                ? t("mail.reader.actions.expungeTitle")
                : t("mail.reader.actions.expungeUnavailableTitle")
            }
            aria-label={
              mailActionAvailable
                ? t("mail.reader.actions.expunge")
                : t("mail.reader.actions.expungeUnavailable")
            }
            onClick={() =>
              onRequestMailAction(
                mailActionRequestFor(message, { kind: "expunge-deleted" }, false),
              )
            }
          >
            {t("mail.reader.actions.expunge")}
          </button>
        ) : null}
        <button
          type="button"
          disabled={
            !messageHasRemoteContent ||
            remoteContentAllowed ||
            !allowRemoteContent ||
            remoteContentBusy
          }
          title={remoteContentActionTitle({
            allowRemoteContentAvailable: Boolean(allowRemoteContent),
            allowedScope: remoteContentAllowedScope,
            hasRemoteContent: messageHasRemoteContent,
            t,
          })}
          aria-label={remoteContentActionLabel({
            allowRemoteContentAvailable: Boolean(allowRemoteContent),
            allowedScope: remoteContentAllowedScope,
            hasRemoteContent: messageHasRemoteContent,
            t,
          })}
          onClick={() => setRemoteContentPanelOpen((current) => !current)}
        >
          {remoteContentBusy
            ? t("mail.reader.remoteContent.loading")
            : t("mail.reader.actions.loadRemote")}
        </button>
      </div>

      <article className="reader-body">
        {mailActionConfirmation ? (
          <MailActionConfirmationPanel
            action={mailActionConfirmation.action}
            busy={mailActionBusy}
            onCancel={onCancelMailAction}
            onConfirm={onConfirmMailAction}
          />
        ) : null}
        {remoteContentPanelOpen &&
        messageHasRemoteContent &&
        !remoteContentAllowed ? (
          <RemoteContentPreferencePanel
            busy={remoteContentBusy}
            canAllowSender={Boolean(message.senderEmail.trim())}
            onAllowMessage={() =>
              void onAllowRemoteContent(message, "message").then(() =>
                setRemoteContentPanelOpen(false),
              )
            }
            onAllowSender={() =>
              void onAllowRemoteContent(message, "sender").then(() =>
                setRemoteContentPanelOpen(false),
              )
            }
            onCancel={() => setRemoteContentPanelOpen(false)}
          />
        ) : null}
        <SafeEmailBody
          allowRemoteContent={remoteContentAllowed}
          bodyHtml={message.bodyHtml}
          bodyText={message.bodyText}
          onOpenExternalLink={(link) => {
            void onOpenExternalLink(link);
          }}
        />
        {externalLinkStatusMessage ? (
          <p
            className={
              externalLinkAction.status === "error"
                ? "external-link-status external-link-status--error"
                : "external-link-status"
            }
            role={externalLinkAction.status === "error" ? "alert" : "status"}
          >
            {externalLinkStatusMessage}
          </p>
        ) : null}
        {remoteContentStatusMessage ? (
          <p
            className={
              remoteContentAction.status === "error"
                ? "remote-content-status remote-content-status--error"
                : "remote-content-status"
            }
            role={remoteContentAction.status === "error" ? "alert" : "status"}
          >
            {remoteContentStatusMessage}
          </p>
        ) : null}
        <MailActionStatus
          action={mailAction}
          onRetry={onRetryMailAction}
          onUndo={onUndoMailAction}
        />
        <ComposerRouteStatus action={composerRouteAction} />
        <AttachmentList
          action={attachmentAction}
          attachments={message.attachments}
          canDownload={Boolean(downloadAttachment)}
          canPreview={Boolean(previewAttachment)}
          canSave={Boolean(saveAttachment)}
          message={message}
          onDownloadAttachment={onDownloadAttachment}
          onPreviewAttachment={onPreviewAttachment}
          onSaveAttachment={onSaveAttachment}
        />
        <AttachmentPreviewPanel
          onClose={onCloseAttachmentPreview}
          preview={attachmentPreview}
        />
      </article>

      <ReaderStatus
        text={
          remoteContentAllowed
            ? t(
                remoteContentAllowedScope === "sender"
                  ? "mail.reader.remoteContent.allowedSenderStatus"
                  : "mail.reader.remoteContent.allowedMessageStatus",
              )
            : t("mail.reader.remoteBlocked")
        }
      />
    </>
  );
}

function remoteContentActionLabel({
  allowRemoteContentAvailable,
  allowedScope,
  hasRemoteContent,
  t,
}: {
  allowRemoteContentAvailable: boolean;
  allowedScope: RemoteContentPreferenceScope | null;
  hasRemoteContent: boolean;
  t: TFunction;
}) {
  if (allowedScope) {
    return t(
      allowedScope === "sender"
        ? "mail.reader.actions.loadRemoteAllowedSender"
        : "mail.reader.actions.loadRemoteAllowedMessage",
    );
  }

  if (!hasRemoteContent) {
    return t("mail.reader.actions.loadRemoteNoContent");
  }

  if (!allowRemoteContentAvailable) {
    return t("mail.reader.actions.loadRemoteUnavailable");
  }

  return t("mail.reader.actions.loadRemote");
}

function MailActionConfirmationPanel({
  action,
  busy,
  onCancel,
  onConfirm,
}: {
  action: "delete-to-trash" | "expunge-deleted";
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  const destructive = action === "expunge-deleted";

  return (
    <section
      className={
        destructive
          ? "mail-action-panel mail-action-panel--destructive"
          : "mail-action-panel"
      }
      aria-label={t(`mail.reader.mailActions.confirm.${action}.aria`)}
    >
      <div>
        <h3>{t(`mail.reader.mailActions.confirm.${action}.heading`)}</h3>
        <p>{t(`mail.reader.mailActions.confirm.${action}.body`)}</p>
      </div>
      <div className="mail-action-panel__actions">
        <button type="button" disabled={busy} onClick={onConfirm}>
          {t(`mail.reader.mailActions.confirm.${action}.confirm`)}
        </button>
        <button type="button" disabled={busy} onClick={onCancel}>
          {t("common.actions.cancel")}
        </button>
      </div>
    </section>
  );
}

function MailActionStatus({
  action,
  onRetry,
  onUndo,
}: {
  action: MailActionUiState;
  onRetry: () => void;
  onUndo: () => void;
}) {
  const { t } = useTranslation();

  if (!action.message) {
    return null;
  }

  return (
    <section
      className={
        action.status === "error"
          ? "mail-action-status mail-action-status--error"
          : "mail-action-status"
      }
      role={action.status === "error" ? "alert" : "status"}
      aria-label={t("mail.reader.mailActions.statusAria")}
    >
      <span>{action.message}</span>
      {action.undo ? (
        <button type="button" onClick={onUndo}>
          {t("mail.reader.mailActions.undo")}
        </button>
      ) : null}
      {action.retryRequest ? (
        <button type="button" onClick={onRetry}>
          {t("common.actions.retry")}
        </button>
      ) : null}
    </section>
  );
}

function ComposerRouteStatus({ action }: { action: ComposerRouteActionState }) {
  if (!action.message) {
    return null;
  }

  return (
    <p
      className={
        action.status === "error"
          ? "mail-action-status mail-action-status--error"
          : "mail-action-status"
      }
      role={action.status === "error" ? "alert" : "status"}
    >
      {action.message}
    </p>
  );
}

function remoteContentActionTitle({
  allowRemoteContentAvailable,
  allowedScope,
  hasRemoteContent,
  t,
}: {
  allowRemoteContentAvailable: boolean;
  allowedScope: RemoteContentPreferenceScope | null;
  hasRemoteContent: boolean;
  t: TFunction;
}) {
  if (allowedScope) {
    return t(
      allowedScope === "sender"
        ? "mail.reader.actions.loadRemoteAllowedSenderTitle"
        : "mail.reader.actions.loadRemoteAllowedMessageTitle",
    );
  }

  if (!hasRemoteContent) {
    return t("mail.reader.actions.loadRemoteNoContentTitle");
  }

  if (!allowRemoteContentAvailable) {
    return t("mail.reader.actions.loadRemoteUnavailableTitle");
  }

  return t("mail.reader.actions.loadRemoteTitle");
}

function RemoteContentPreferencePanel({
  busy,
  canAllowSender,
  onAllowMessage,
  onAllowSender,
  onCancel,
}: {
  busy: boolean;
  canAllowSender: boolean;
  onAllowMessage: () => void;
  onAllowSender: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();

  return (
    <section
      className="remote-content-panel"
      aria-label={t("mail.reader.remoteContent.panelAria")}
    >
      <div>
        <h3>{t("mail.reader.remoteContent.heading")}</h3>
        <p>{t("mail.reader.remoteContent.body")}</p>
      </div>
      <div
        className="remote-content-panel__actions"
        aria-label={t("mail.reader.remoteContent.actionsAria")}
      >
        <button type="button" disabled={busy} onClick={onAllowMessage}>
          {t("mail.reader.remoteContent.allowMessage")}
        </button>
        <button
          type="button"
          disabled={busy || !canAllowSender}
          title={
            canAllowSender
              ? t("mail.reader.remoteContent.allowSenderTitle")
              : t("mail.reader.remoteContent.allowSenderUnavailableTitle")
          }
          onClick={onAllowSender}
        >
          {t("mail.reader.remoteContent.allowSender")}
        </button>
        <button type="button" disabled={busy} onClick={onCancel}>
          {t("common.actions.cancel")}
        </button>
      </div>
    </section>
  );
}

function AttachmentList({
  action,
  attachments,
  canDownload,
  canPreview,
  canSave,
  message,
  onDownloadAttachment,
  onPreviewAttachment,
  onSaveAttachment,
}: {
  action: AttachmentActionState;
  attachments: MailAttachment[];
  canDownload: boolean;
  canPreview: boolean;
  canSave: boolean;
  message: MailMessage;
  onDownloadAttachment: (message: MailMessage, attachment: MailAttachment) => void;
  onPreviewAttachment: (message: MailMessage, attachment: MailAttachment) => void;
  onSaveAttachment: (message: MailMessage, attachment: MailAttachment) => void;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();

  if (attachments.length === 0) {
    return null;
  }

  return (
    <section className="attachment-list" aria-label={t("mail.attachments.heading")}>
      <h3>{t("mail.attachments.heading")}</h3>
      <ul>
        {attachments.map((attachment) => {
          const displayFilename =
            attachment.displayFilename.trim() ||
            t("mail.attachments.attachmentFallback");
          const contentType =
            attachment.contentType.trim() || "application/octet-stream";
          const isBusy =
            action.attachmentId === attachment.id && action.status === "loading";
          const statusMessage =
            action.attachmentId === attachment.id ? action.message : null;
          const canPreviewThisAttachment =
            canPreview && canPreviewAttachmentMetadata(attachment);

          return (
            <li className="attachment-item" key={attachment.id}>
              <div className="attachment-meta">
                <strong>{displayFilename}</strong>
                <span>
                  {contentType} - {localeFormat.byteSize(attachment.sizeBytes)}
                </span>
                <small>{t(`mail.attachments.disposition.${attachment.disposition}`)}</small>
              </div>
              <div className="attachment-actions" aria-label={t("mail.attachments.actionsLabel")}>
                <button
                  type="button"
                  disabled={!canPreviewThisAttachment || isBusy}
                  title={
                    canPreviewThisAttachment
                      ? t("mail.attachments.previewTitle")
                      : canPreview
                        ? t("mail.attachments.previewUnsupportedTitle")
                      : t("mail.attachments.previewUnavailableTitle")
                  }
                  aria-label={
                    canPreviewThisAttachment
                      ? t("mail.attachments.previewNamed", {
                          filename: displayFilename,
                        })
                      : t("mail.attachments.previewUnavailable", {
                          filename: displayFilename,
                        })
                  }
                  onClick={() => onPreviewAttachment(message, attachment)}
                >
                  {t("mail.attachments.preview")}
                </button>
                <button
                  type="button"
                  disabled={!canSave || isBusy}
                  title={
                    canSave
                      ? t("mail.attachments.saveTitle")
                      : t("mail.attachments.saveUnavailableTitle")
                  }
                  aria-label={
                    canSave
                      ? t("mail.attachments.saveNamed", {
                          filename: displayFilename,
                        })
                      : t("mail.attachments.saveUnavailable", {
                          filename: displayFilename,
                        })
                  }
                  onClick={() => onSaveAttachment(message, attachment)}
                >
                  {t("mail.attachments.save")}
                </button>
                <button
                  type="button"
                  disabled={!canDownload || isBusy}
                  title={
                    canDownload
                      ? t("mail.attachments.downloadTitle")
                      : t("mail.attachments.downloadUnavailableTitle")
                  }
                  aria-label={
                    canDownload
                      ? t("mail.attachments.downloadNamed", {
                          filename: displayFilename,
                        })
                      : t("mail.attachments.downloadUnavailable", {
                          filename: displayFilename,
                        })
                  }
                  onClick={() => onDownloadAttachment(message, attachment)}
                >
                  {t("mail.attachments.download")}
                </button>
              </div>
              {isBusy ? (
                <small className="attachment-status" role="status">
                  {t("mail.attachments.working")}
                </small>
              ) : null}
              {statusMessage ? (
                <small
                  className={
                    action.status === "error"
                      ? "attachment-status attachment-status--error"
                      : "attachment-status"
                  }
                  role="status"
                >
                  {statusMessage}
                </small>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function ReaderStatus({ text }: { text: string }) {
  const { t } = useTranslation();

  return (
    <footer className="reader-status">
      <span>{text}</span>
      <span>{t("common.status.localCache")}</span>
    </footer>
  );
}

function AttachmentPreviewPanel({
  onClose,
  preview,
}: {
  onClose: () => void;
  preview: AttachmentPreviewResponse | null;
}) {
  const { t } = useTranslation();

  if (!preview) {
    return null;
  }

  const imageUrl =
    preview.kind === "raster-image" && preview.bytes
      ? bytesToDataUrl(preview.contentType, preview.bytes)
      : null;

  return (
    <section className="attachment-preview" aria-label={t("mail.attachments.previewHeading")}>
      <header>
        <div>
          <p className="eyebrow">{t("mail.attachments.previewEyebrow")}</p>
          <h3>{preview.displayFilename}</h3>
        </div>
        <button type="button" onClick={onClose}>
          {t("mail.attachments.closePreview")}
        </button>
      </header>
      {preview.kind === "plain-text" ? (
        <pre>{preview.text}</pre>
      ) : imageUrl ? (
        <img src={imageUrl} alt={preview.displayFilename} />
      ) : null}
    </section>
  );
}
