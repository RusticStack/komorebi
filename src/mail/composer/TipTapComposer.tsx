import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useLocaleFormatters } from "../../i18n";
import {
  FONT_FAMILIES,
  FONT_SIZES,
  HIGHLIGHT_COLORS,
  TEXT_COLORS,
  allowedFontFamilies,
  allowedFontSizes,
  allowedStyleColors,
  composerEditorExtensions,
  safeComposerLink,
  sanitizeComposerHtml,
  type AlignmentValue,
} from "./composerRichText";
import {
  insertSignatureIntoComposerHtml,
} from "../signatures/signatureHtml";
import type {
  ComposerAddress,
  ComposerAttachment,
  ComposerAttachmentDisposition,
  ComposerAttachmentStageRequest,
  ComposerDraft,
  ComposerDraftDiscardRequest,
  ComposerDraftSaveRequest,
  ComposerDraftSaveResponse,
  ComposerSendRequest,
  ComposerSendResponse,
  MailAccount,
  RecentRecipientSearchRequest,
  RecentRecipientSuggestion,
} from "../shell/mailShellState";

export type ComposerDraftSaveFunction = (
  request: ComposerDraftSaveRequest,
) => Promise<ComposerDraftSaveResponse>;

export type ComposerDraftDiscardFunction = (
  request: ComposerDraftDiscardRequest,
) => Promise<ComposerDraftSaveResponse>;

export type ComposerAttachmentStageFunction = (
  request: ComposerAttachmentStageRequest,
) => Promise<ComposerAttachment>;

export type ComposerSendFunction = (
  request: ComposerSendRequest,
) => Promise<ComposerSendResponse>;

export type RecentRecipientSearchFunction = (
  request: RecentRecipientSearchRequest,
  signal: AbortSignal,
) => Promise<RecentRecipientSuggestion[]>;

export type TipTapComposerProps = {
  accounts: MailAccount[];
  initialDraft?: ComposerDraft | null;
  initialAccountId: string | null;
  loadRecentRecipients?: RecentRecipientSearchFunction;
  saveDraft?: ComposerDraftSaveFunction;
  discardDraft?: ComposerDraftDiscardFunction;
  stageAttachment?: ComposerAttachmentStageFunction;
  sendMail?: ComposerSendFunction;
  onDraftDiscarded?: (draftId: string) => void;
  onDraftSaved?: (draftId: string) => void;
  onDraftSent?: (draftId: string) => void;
  onClose: () => void;
};

type DraftStatus = "idle" | "dirty" | "saving" | "saved" | "sent" | "error";
type SendStatus = "idle" | "validating" | "sending" | "sent" | "error";
type ParsedAddressList = {
  addresses: ComposerAddress[];
  errors: string[];
};
type RecipientFieldName = "to" | "cc" | "bcc";
type ParsedRecipientFields = {
  to: ComposerAddress[];
  cc: ComposerAddress[];
  bcc: ComposerAddress[];
  errors: string[];
  duplicateNotices: string[];
};

const AUTOSAVE_DEBOUNCE_MS = 700;
const RECIPIENT_AUTOCOMPLETE_DEBOUNCE_MS = 150;

export function TipTapComposer({
  accounts,
  initialDraft = null,
  initialAccountId,
  loadRecentRecipients,
  saveDraft,
  discardDraft,
  stageAttachment,
  sendMail,
  onDraftDiscarded,
  onDraftSaved,
  onDraftSent,
  onClose,
}: TipTapComposerProps) {
  const { t } = useTranslation();
  const [accountId, setAccountId] = useState(
    initialDraft?.accountId ?? initialAccountId ?? accounts[0]?.id ?? "",
  );
  const [draftId] = useState(() => initialDraft?.id ?? createDraftId());
  const [toInput, setToInput] = useState(() =>
    formatComposerAddresses(initialDraft?.to ?? []),
  );
  const [ccInput, setCcInput] = useState(() =>
    formatComposerAddresses(initialDraft?.cc ?? []),
  );
  const [bccInput, setBccInput] = useState(() =>
    formatComposerAddresses(initialDraft?.bcc ?? []),
  );
  const [subject, setSubject] = useState(initialDraft?.subject ?? "");
  const [bodyHtml, setBodyHtml] = useState(initialDraft?.bodyHtml ?? "<p></p>");
  const [bodyText, setBodyText] = useState(initialDraft?.bodyText ?? "");
  const [linkInput, setLinkInput] = useState("");
  const [attachments, setAttachments] = useState<ComposerAttachment[]>(
    initialDraft?.attachments ?? [],
  );
  const [inlineImageSources, setInlineImageSources] = useState(
    () => new Map<string, string>(),
  );
  const [validationErrors, setValidationErrors] = useState<string[]>([]);
  const [recipientNotices, setRecipientNotices] = useState<string[]>([]);
  const [draftStatus, setDraftStatus] = useState<DraftStatus>(
    initialDraft ? "saved" : "idle",
  );
  const [sendStatus, setSendStatus] = useState<SendStatus>("idle");
  const [progress, setProgress] = useState<string[]>([]);
  const attachmentInputRef = useRef<HTMLInputElement | null>(null);
  const inlineImageInputRef = useRef<HTMLInputElement | null>(null);
  const activeAccount = accounts.find((account) => account.id === accountId);
  const canUseBackend = Boolean(saveDraft && stageAttachment && sendMail);
  const editor = useEditor({
    extensions: composerEditorExtensions,
    content: bodyHtml,
    editorProps: {
      attributes: {
        "aria-label": t("composer.aria.body"),
        "aria-multiline": "true",
        class: "composer-editor",
        role: "textbox",
      },
    },
    immediatelyRender: false,
    onUpdate: ({ editor }) => {
      setBodyHtml(editor.getHTML());
      setBodyText(editor.getText());
      setDraftStatus((current) => (current === "sent" ? current : "dirty"));
    },
  });

  const attachmentIds = useMemo(
    () => attachments.map((attachment) => attachment.id),
    [attachments],
  );

  useEffect(() => {
    if (!saveDraft || !accountId || sendStatus === "sent") {
      return;
    }

    const handle = window.setTimeout(() => {
      void saveDraftNow({ showErrors: false });
    }, AUTOSAVE_DEBOUNCE_MS);

    return () => {
      window.clearTimeout(handle);
    };
    // saveDraftNow is intentionally omitted so this timer follows field changes,
    // not helper identity churn across renders.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    accountId,
    attachmentIds,
    bccInput,
    bodyHtml,
    bodyText,
    ccInput,
    saveDraft,
    sendStatus,
    subject,
    toInput,
  ]);

  async function saveDraftNow({
    showErrors,
  }: {
    showErrors: boolean;
  }): Promise<ComposerDraftSaveResponse | null> {
    if (!saveDraft) {
      if (showErrors) {
        setValidationErrors([t("composer.errors.draftSaveUnavailable")]);
        setDraftStatus("error");
      }
      return null;
    }

    const request = buildDraftRequest({ requireSendReady: false });
    if (request.errors.length > 0) {
      if (showErrors) {
        setValidationErrors(request.errors);
        setRecipientNotices(request.duplicateNotices);
        setDraftStatus("error");
      }
      return null;
    }

    setDraftStatus("saving");
    try {
      const saved = await saveDraft(request.value);
      setDraftStatus("saved");
      onDraftSaved?.(saved.id);
      if (showErrors) {
        setValidationErrors([]);
        setRecipientNotices(request.duplicateNotices);
      }
      return saved;
    } catch {
      setDraftStatus("error");
      if (showErrors) {
        setValidationErrors([t("composer.errors.draftCouldNotSave")]);
      }
      return null;
    }
  }

  async function handleDiscardDraft() {
    if (!discardDraft || !accountId) {
      onClose();
      return;
    }

    try {
      await discardDraft({ accountId, draftId });
      onDraftDiscarded?.(draftId);
    } catch {
      setValidationErrors([t("composer.errors.draftCouldNotDiscard")]);
      setDraftStatus("error");
      return;
    }

    onClose();
  }

  async function handleStageFiles(
    files: FileList | null,
    disposition: ComposerAttachmentDisposition,
  ) {
    if (!files || files.length === 0) {
      return;
    }
    if (!stageAttachment) {
      setValidationErrors([t("composer.errors.stageUnavailable")]);
      return;
    }

    const saved = await saveDraftNow({ showErrors: true });
    if (!saved) {
      return;
    }

    const stagedAttachments: ComposerAttachment[] = [];
    const nextInlineSources = new Map(inlineImageSources);
    for (const file of Array.from(files)) {
      try {
        const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
        const request: ComposerAttachmentStageRequest = {
          accountId,
          bytes,
          contentType: file.type || null,
          displayFilename: file.name,
          disposition,
          draftId,
        };
        const staged = await stageAttachment(request);
        stagedAttachments.push(staged);

        if (disposition === "inline" && staged.contentId && editor) {
          const dataUrl = await fileToDataUrl(file);
          nextInlineSources.set(dataUrl, staged.contentId);
          editor
            .chain()
            .focus()
            .setImage({ alt: staged.displayFilename, src: dataUrl })
            .run();
        }
      } catch {
        setValidationErrors([
          t("composer.errors.attachmentCouldNotStage", {
            filename: file.name || t("composer.errors.unknownAttachment"),
          }),
        ]);
        return;
      }
    }

    setInlineImageSources(nextInlineSources);
    setAttachments((current) => [...current, ...stagedAttachments]);
    setDraftStatus("dirty");
    setValidationErrors([]);
  }

  function removeAttachment(attachmentId: string) {
    setAttachments((current) =>
      current.filter((attachment) => attachment.id !== attachmentId),
    );
    setDraftStatus("dirty");
  }

  function insertSignature() {
    if (!editor || !activeAccount?.signatureHtml) {
      return;
    }

    const signedHtml = insertSignatureIntoComposerHtml(
      editor.getHTML(),
      activeAccount.signatureHtml,
    );
    editor.commands.setContent(signedHtml);
    setBodyHtml(editor.getHTML());
    setBodyText(editor.getText());
    setDraftStatus("dirty");
  }

  function applyLink() {
    if (!editor) {
      return;
    }

    const parsed = safeComposerLink(linkInput);
    if (!parsed) {
      setValidationErrors([t("composer.errors.linkUnsafe")]);
      return;
    }

    editor.chain().focus().extendMarkRange("link").setLink({ href: parsed.href }).run();
    setLinkInput("");
    setValidationErrors([]);
  }

  function applyFontFamily(value: string) {
    if (!editor) {
      return;
    }
    if (!value) {
      editor.chain().focus().unsetFontFamily().run();
      return;
    }
    if (allowedFontFamilies.has(value)) {
      editor.chain().focus().setFontFamily(value).run();
    }
  }

  function applyFontSize(value: string) {
    if (!editor) {
      return;
    }
    if (!value) {
      editor.chain().focus().unsetFontSize().run();
      return;
    }
    if (allowedFontSizes.has(value)) {
      editor.chain().focus().setFontSize(value).run();
    }
  }

  function applyTextColor(value: string) {
    if (!editor) {
      return;
    }
    if (!value) {
      editor.chain().focus().unsetColor().run();
      return;
    }
    if (allowedStyleColors.has(value)) {
      editor.chain().focus().setColor(value).run();
    }
  }

  function applyHighlight(value: string) {
    if (!editor) {
      return;
    }
    if (!value) {
      editor.chain().focus().unsetHighlight().run();
      return;
    }
    if (allowedStyleColors.has(value)) {
      editor.chain().focus().setHighlight({ color: value }).run();
    }
  }

  function applyAlignment(value: AlignmentValue) {
    editor?.chain().focus().setTextAlign(value).run();
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!sendMail) {
      setValidationErrors([t("composer.errors.sendingUnavailable")]);
      setSendStatus("error");
      return;
    }

    setSendStatus("validating");
    const request = buildDraftRequest({ requireSendReady: true });
    if (request.errors.length > 0) {
      setValidationErrors(request.errors);
      setRecipientNotices(request.duplicateNotices);
      setSendStatus("error");
      return;
    }

    setValidationErrors([]);
    setRecipientNotices(request.duplicateNotices);
    setProgress([t("composer.status.savingDraft"), t("composer.status.sending")]);
    const saved = await saveDraftNow({ showErrors: true });
    if (!saved) {
      setSendStatus("error");
      return;
    }

    setSendStatus("sending");
    try {
      const response = await sendMail(request.value);
      setSendStatus("sent");
      setDraftStatus("sent");
      setProgress(response.progress.map((item) => sendProgressLabel(item, t)));
      onDraftSent?.(response.draftId);
    } catch {
      setSendStatus("error");
      setProgress([]);
      setValidationErrors([t("composer.errors.sendFailed")]);
    }
  }

  function buildDraftRequest({
    requireSendReady,
  }: {
    requireSendReady: boolean;
  }):
    | {
        value: ComposerDraftSaveRequest;
        errors: [];
        duplicateNotices: string[];
      }
    | {
        value: ComposerDraftSaveRequest;
        errors: string[];
        duplicateNotices: string[];
      } {
    const recipients = parseRecipientFields(toInput, ccInput, bccInput, t);
    const errors = [...recipients.errors];
    const sanitizedHtml = sanitizeComposerHtml(
      editor?.getHTML() ?? bodyHtml,
      inlineImageSources,
    );
    const cleanedBodyText = (editor?.getText() ?? bodyText).trim();
    const htmlHasContent = composerHtmlHasContent(sanitizedHtml);

    if (!accountId) {
      errors.push(t("composer.errors.accountRequired"));
    }
    if (
      requireSendReady &&
      recipients.to.length + recipients.cc.length + recipients.bcc.length === 0
    ) {
      errors.push(t("composer.errors.recipientRequired"));
    }
    if (requireSendReady && !cleanedBodyText && !htmlHasContent) {
      errors.push(t("composer.errors.bodyRequired"));
    }

    return {
      duplicateNotices: recipients.duplicateNotices,
      errors,
      value: {
        accountId,
        attachmentIds,
        bcc: recipients.bcc,
        bodyHtml: htmlHasContent ? sanitizedHtml : null,
        bodyText: cleanedBodyText || null,
        cc: recipients.cc,
        draftId,
        subject,
        to: recipients.to,
      },
    };
  }

  if (accounts.length === 0) {
    return (
      <section className="composer-panel" aria-label={t("composer.aria.composer")}>
        <header className="settings-panel__header">
          <div>
            <p className="eyebrow">{t("composer.aria.composer")}</p>
            <h2>{t("composer.heading")}</h2>
          </div>
          <button type="button" onClick={onClose}>
            {t("common.actions.close")}
          </button>
        </header>
        <section className="empty-panel" aria-label={t("composer.aria.unavailable")}>
          <h3>{t("composer.empty.heading")}</h3>
          <p>{t("composer.empty.body")}</p>
        </section>
      </section>
    );
  }

  return (
    <section className="composer-panel" aria-label={t("composer.aria.composer")}>
      <header className="settings-panel__header">
        <div>
          <p className="eyebrow">{activeAccount?.email ?? t("composer.aria.composer")}</p>
          <h2>{t("composer.heading")}</h2>
        </div>
        <button type="button" onClick={onClose}>
          {t("common.actions.close")}
        </button>
      </header>

      <form className="composer-form" onSubmit={handleSubmit}>
        <label>
          <span>{t("composer.fields.from")}</span>
          <select
            value={accountId}
            disabled={Boolean(initialDraft) || sendStatus === "sending" || sendStatus === "sent"}
            onChange={(event) => {
              setAccountId(event.target.value);
              setRecipientNotices([]);
              setDraftStatus("dirty");
            }}
          >
            {accounts.map((account) => (
              <option key={account.id} value={account.id}>
                {account.label} ({account.email})
              </option>
            ))}
          </select>
        </label>

        <div className="composer-recipient-grid">
          <RecipientAutocompleteField
            accountId={accountId}
            autoFocus
            label={t("composer.fields.to")}
            loadRecentRecipients={loadRecentRecipients}
            value={toInput}
            onChange={(value) => {
              setToInput(value);
              setRecipientNotices([]);
              setDraftStatus("dirty");
            }}
          />
          <RecipientAutocompleteField
            accountId={accountId}
            label={t("composer.fields.cc")}
            loadRecentRecipients={loadRecentRecipients}
            value={ccInput}
            onChange={(value) => {
              setCcInput(value);
              setRecipientNotices([]);
              setDraftStatus("dirty");
            }}
          />
          <RecipientAutocompleteField
            accountId={accountId}
            label={t("composer.fields.bcc")}
            loadRecentRecipients={loadRecentRecipients}
            value={bccInput}
            onChange={(value) => {
              setBccInput(value);
              setRecipientNotices([]);
              setDraftStatus("dirty");
            }}
          />
        </div>

        <label>
          <span>{t("composer.fields.subject")}</span>
          <input
            type="text"
            value={subject}
            onChange={(event) => {
              setSubject(event.target.value);
              setDraftStatus("dirty");
            }}
          />
        </label>

        <div className="composer-toolbar" aria-label={t("composer.aria.formattingControls")}>
          <FormatButton editor={editor} command="bold" label={t("composer.format.bold")} />
          <FormatButton editor={editor} command="italic" label={t("composer.format.italic")} />
          <FormatButton editor={editor} command="strike" label={t("composer.format.strikethrough")} />
          <FormatButton editor={editor} command="code" label={t("composer.format.inlineCode")} />
          <FormatButton editor={editor} command="codeBlock" label={t("composer.format.codeBlock")} />
          <FormatButton editor={editor} command="bulletList" label={t("composer.format.bulletedList")} />
          <FormatButton editor={editor} command="orderedList" label={t("composer.format.numberedList")} />
          <FormatButton editor={editor} command="blockquote" label={t("composer.format.blockquote")} />
          <label className="composer-compact-control">
            <span>{t("composer.format.fontFamily")}</span>
            <select
              value=""
              onChange={(event) => applyFontFamily(event.target.value)}
              disabled={!editor}
            >
              <option value="">{t("composer.format.default")}</option>
              {FONT_FAMILIES.map((option) => (
                <option key={option.value} value={option.value}>
                  {t(option.labelKey)}
                </option>
              ))}
            </select>
          </label>
          <label className="composer-compact-control">
            <span>{t("composer.format.fontSize")}</span>
            <select
              value=""
              onChange={(event) => applyFontSize(event.target.value)}
              disabled={!editor}
            >
              <option value="">{t("composer.format.default")}</option>
              {FONT_SIZES.map((size) => (
                <option key={size} value={size}>
                  {size}
                </option>
              ))}
            </select>
          </label>
          <label className="composer-compact-control">
            <span>{t("composer.format.textColor")}</span>
            <select
              value=""
              onChange={(event) => applyTextColor(event.target.value)}
              disabled={!editor}
            >
              <option value="">{t("composer.format.default")}</option>
              {TEXT_COLORS.map((option) => (
                <option key={option.value} value={option.value}>
                  {t(option.labelKey)}
                </option>
              ))}
            </select>
          </label>
          <label className="composer-compact-control">
            <span>{t("composer.format.highlight")}</span>
            <select
              value=""
              onChange={(event) => applyHighlight(event.target.value)}
              disabled={!editor}
            >
              <option value="">{t("composer.format.none")}</option>
              {HIGHLIGHT_COLORS.map((option) => (
                <option key={option.value} value={option.value}>
                  {t(option.labelKey)}
                </option>
              ))}
            </select>
          </label>
          <div className="composer-button-group" aria-label={t("composer.format.alignmentLabel")}>
            <button type="button" onClick={() => applyAlignment("left")} disabled={!editor}>
              {t("composer.format.alignLeft")}
            </button>
            <button type="button" onClick={() => applyAlignment("center")} disabled={!editor}>
              {t("composer.format.alignCenter")}
            </button>
            <button type="button" onClick={() => applyAlignment("right")} disabled={!editor}>
              {t("composer.format.alignRight")}
            </button>
          </div>
          <div className="composer-button-group" aria-label={t("composer.format.table.controlsLabel")}>
            <button
              type="button"
              onClick={() =>
                editor
                  ?.chain()
                  .focus()
                  .insertTable({ cols: 2, rows: 2, withHeaderRow: true })
                  .run()
              }
              disabled={!editor}
            >
              {t("composer.format.table.insert")}
            </button>
            <button
              type="button"
              onClick={() => editor?.chain().focus().addRowAfter().run()}
              disabled={!editor || !editor.isActive("table")}
            >
              {t("composer.format.table.addRow")}
            </button>
            <button
              type="button"
              onClick={() => editor?.chain().focus().addColumnAfter().run()}
              disabled={!editor || !editor.isActive("table")}
            >
              {t("composer.format.table.addColumn")}
            </button>
            <button
              type="button"
              onClick={() => editor?.chain().focus().deleteTable().run()}
              disabled={!editor || !editor.isActive("table")}
            >
              {t("composer.format.table.delete")}
            </button>
          </div>
          <label className="composer-link-control">
            <span>{t("composer.fields.linkUrl")}</span>
            <input
              type="url"
              value={linkInput}
              onChange={(event) => setLinkInput(event.target.value)}
            />
          </label>
          <button type="button" onClick={applyLink} disabled={!editor}>
            {t("composer.actions.applyLink")}
          </button>
          <button
            type="button"
            onClick={() => editor?.chain().focus().unsetLink().run()}
            disabled={!editor}
          >
            {t("composer.actions.removeLink")}
          </button>
          <button
            type="button"
            onClick={() => attachmentInputRef.current?.click()}
            disabled={!canUseBackend || sendStatus === "sending"}
          >
            {t("composer.actions.attach")}
          </button>
          <button
            type="button"
            onClick={() => inlineImageInputRef.current?.click()}
            disabled={!canUseBackend || sendStatus === "sending"}
          >
            {t("composer.actions.inlineImage")}
          </button>
          <button
            type="button"
            onClick={insertSignature}
            disabled={!editor || !activeAccount?.signatureHtml}
          >
            {t("composer.actions.insertSignature")}
          </button>
          <input
            aria-label={t("composer.aria.attachmentPicker")}
            className="visually-hidden"
            multiple
            ref={attachmentInputRef}
            type="file"
            onChange={(event) => {
              void handleStageFiles(event.target.files, "attachment");
              event.target.value = "";
            }}
          />
          <input
            accept="image/png,image/jpeg,image/gif,image/webp"
            aria-label={t("composer.aria.inlineImagePicker")}
            className="visually-hidden"
            ref={inlineImageInputRef}
            type="file"
            onChange={(event) => {
              void handleStageFiles(event.target.files, "inline");
              event.target.value = "";
            }}
          />
        </div>

        <EditorContent editor={editor} />

        <ComposerAttachmentList
          attachments={attachments}
          onRemove={removeAttachment}
        />

        <ComposerStatus
          draftStatus={draftStatus}
          progress={progress}
          sendStatus={sendStatus}
        />

        {validationErrors.length > 0 ? (
          <div className="composer-errors" role="alert">
            {validationErrors.map((error) => (
              <p key={error}>{error}</p>
            ))}
          </div>
        ) : null}

        {recipientNotices.length > 0 ? (
          <div className="composer-notices" role="status">
            {recipientNotices.map((notice) => (
              <p key={notice}>{notice}</p>
            ))}
          </div>
        ) : null}

        <div className="settings-panel__actions">
          <button
            type="button"
            onClick={() => void saveDraftNow({ showErrors: true })}
            disabled={sendStatus === "sending" || sendStatus === "sent"}
          >
            {t("composer.actions.saveDraft")}
          </button>
          <button
            type="button"
            onClick={() => void handleDiscardDraft()}
            disabled={sendStatus === "sending" || sendStatus === "sent"}
          >
            {t("composer.actions.discard")}
          </button>
          <button
            type="submit"
            disabled={sendStatus === "sending" || sendStatus === "sent"}
          >
            {t("composer.actions.send")}
          </button>
        </div>
      </form>
    </section>
  );
}

function FormatButton({
  command,
  editor,
  label,
}: {
  command:
    | "bold"
    | "italic"
    | "strike"
    | "code"
    | "codeBlock"
    | "bulletList"
    | "orderedList"
    | "blockquote";
  editor: Editor | null;
  label: string;
}) {
  const active = editor?.isActive(command) ?? false;

  function runCommand() {
    const chain = editor?.chain().focus();
    switch (command) {
      case "bold":
        chain?.toggleBold().run();
        break;
      case "italic":
        chain?.toggleItalic().run();
        break;
      case "strike":
        chain?.toggleStrike().run();
        break;
      case "code":
        chain?.toggleCode().run();
        break;
      case "codeBlock":
        chain?.toggleCodeBlock().run();
        break;
      case "bulletList":
        chain?.toggleBulletList().run();
        break;
      case "orderedList":
        chain?.toggleOrderedList().run();
        break;
      case "blockquote":
        chain?.toggleBlockquote().run();
        break;
    }
  }

  return (
    <button
      aria-pressed={active}
      className={active ? "is-active" : undefined}
      disabled={!editor}
      type="button"
      onClick={runCommand}
    >
      {label}
    </button>
  );
}

function ComposerAttachmentList({
  attachments,
  onRemove,
}: {
  attachments: ComposerAttachment[];
  onRemove: (attachmentId: string) => void;
}) {
  const { t } = useTranslation();
  const localeFormat = useLocaleFormatters();

  if (attachments.length === 0) {
    return null;
  }

  return (
    <section className="composer-attachments" aria-label={t("composer.aria.attachments")}>
      <ul>
        {attachments.map((attachment) => (
          <li key={attachment.id}>
            <span>
              {attachment.displayFilename} - {attachment.contentType} -{" "}
              {localeFormat.byteSize(attachment.sizeBytes)}
            </span>
            <small>{t(`composer.attachments.disposition.${attachment.disposition}`)}</small>
            <button type="button" onClick={() => onRemove(attachment.id)}>
              {t("composer.actions.remove")}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function ComposerStatus({
  draftStatus,
  progress,
  sendStatus,
}: {
  draftStatus: DraftStatus;
  progress: string[];
  sendStatus: SendStatus;
}) {
  const { t } = useTranslation();
  const statusText =
    sendStatus === "sent"
      ? t("composer.status.sent")
      : sendStatus === "sending"
        ? t("composer.status.sending")
        : sendStatus === "error"
          ? t("composer.status.needsAttention")
          : draftStatus === "saved"
            ? t("composer.status.draftSaved")
            : draftStatus === "saving"
              ? t("composer.status.savingDraft")
              : draftStatus === "error"
                ? t("composer.status.draftFailed")
                : t("composer.status.draftNotSaved");

  return (
    <div className="composer-status" role="status">
      <span>{statusText}</span>
      {sendStatus !== "sent" ? (
        <small>{t("composer.status.localOnlyDraft")}</small>
      ) : null}
      {progress.length > 0 ? (
        <ol>
          {progress.map((item) => (
            <li key={item}>{item}</li>
          ))}
        </ol>
      ) : null}
    </div>
  );
}

function RecipientAutocompleteField({
  accountId,
  autoFocus = false,
  label,
  loadRecentRecipients,
  onChange,
  value,
}: {
  accountId: string;
  autoFocus?: boolean;
  label: string;
  loadRecentRecipients?: RecentRecipientSearchFunction;
  onChange: (value: string) => void;
  value: string;
}) {
  const inputId = useId();
  const listboxId = `${inputId}-recent-recipients`;
  const [suggestions, setSuggestions] = useState<RecentRecipientSuggestion[]>([]);
  const [isOpen, setIsOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const query = currentRecipientToken(value);
  const canLoad = Boolean(loadRecentRecipients && accountId);

  useEffect(() => {
    if (!loadRecentRecipients || !accountId || !isOpen) {
      return;
    }

    const controller = new AbortController();
    const handle = window.setTimeout(() => {
      void loadRecentRecipients(
        {
          accountId,
          limit: 8,
          query,
        },
        controller.signal,
      )
        .then((nextSuggestions) => {
          if (!controller.signal.aborted) {
            setSuggestions(nextSuggestions);
            setActiveIndex(0);
          }
        })
        .catch((error: unknown) => {
          if (!isAbortError(error)) {
            setSuggestions([]);
          }
        });
    }, RECIPIENT_AUTOCOMPLETE_DEBOUNCE_MS);

    return () => {
      controller.abort();
      window.clearTimeout(handle);
    };
  }, [accountId, isOpen, loadRecentRecipients, query]);

  const activeSuggestion = suggestions[activeIndex];

  function selectSuggestion(suggestion: RecentRecipientSuggestion) {
    onChange(replaceCurrentRecipientToken(value, suggestion));
    setIsOpen(false);
    setSuggestions([]);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (!isOpen && ["ArrowDown", "ArrowUp"].includes(event.key) && canLoad) {
      setIsOpen(true);
      return;
    }
    if (!suggestions.length) {
      if (event.key === "Escape") {
        setIsOpen(false);
      }
      return;
    }

    if (event.key === "ArrowDown") {
      event.preventDefault();
      setIsOpen(true);
      setActiveIndex((current) => (current + 1) % suggestions.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setIsOpen(true);
      setActiveIndex(
        (current) => (current - 1 + suggestions.length) % suggestions.length,
      );
    } else if (event.key === "Enter" && isOpen && activeSuggestion) {
      event.preventDefault();
      selectSuggestion(activeSuggestion);
    } else if (event.key === "Escape") {
      setIsOpen(false);
    }
  }

  return (
    <label className="recipient-field" htmlFor={inputId}>
      <span>{label}</span>
      <input
        aria-activedescendant={
          isOpen && activeSuggestion
            ? `${listboxId}-option-${activeSuggestion.id}`
            : undefined
        }
        aria-autocomplete="list"
        aria-controls={listboxId}
        aria-expanded={isOpen && suggestions.length > 0}
        autoComplete="off"
        data-autofocus={autoFocus ? "true" : undefined}
        id={inputId}
        role="combobox"
        type="text"
        value={value}
        onBlur={() => {
          window.setTimeout(() => setIsOpen(false), 100);
        }}
        onChange={(event) => {
          onChange(event.target.value);
          setIsOpen(true);
        }}
        onFocus={() => {
          if (canLoad) {
            setIsOpen(true);
          }
        }}
        onKeyDown={handleKeyDown}
      />
      {isOpen && suggestions.length > 0 ? (
        <ul
          className="recipient-suggestions"
          id={listboxId}
          role="listbox"
        >
          {suggestions.map((suggestion, index) => (
            <li key={suggestion.id} role="presentation">
              <button
                aria-selected={index === activeIndex}
                id={`${listboxId}-option-${suggestion.id}`}
                role="option"
                type="button"
                onClick={() => selectSuggestion(suggestion)}
                onMouseDown={(event) => event.preventDefault()}
                onMouseEnter={() => setActiveIndex(index)}
              >
                <span>{suggestion.name ?? suggestion.address}</span>
                {suggestion.name ? <small>{suggestion.address}</small> : null}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </label>
  );
}

function parseRecipientFields(
  toInput: string,
  ccInput: string,
  bccInput: string,
  t: TFunction,
): ParsedRecipientFields {
  const to = parseAddressList("to", toInput, t);
  const cc = parseAddressList("cc", ccInput, t);
  const bcc = parseAddressList("bcc", bccInput, t);
  const seen = new Map<string, RecipientFieldName>();
  const duplicateNotices: string[] = [];
  const dedupedTo = deduplicateParsedAddresses("to", to.addresses, seen, duplicateNotices, t);
  const dedupedCc = deduplicateParsedAddresses("cc", cc.addresses, seen, duplicateNotices, t);
  const dedupedBcc = deduplicateParsedAddresses("bcc", bcc.addresses, seen, duplicateNotices, t);

  return {
    bcc: dedupedBcc,
    cc: dedupedCc,
    duplicateNotices,
    errors: [...to.errors, ...cc.errors, ...bcc.errors],
    to: dedupedTo,
  };
}

function deduplicateParsedAddresses(
  field: RecipientFieldName,
  addresses: ComposerAddress[],
  seen: Map<string, RecipientFieldName>,
  duplicateNotices: string[],
  t: TFunction,
) {
  const deduped: ComposerAddress[] = [];
  for (const address of addresses) {
    const normalized = normalizeEmailAddress(address.address);
    const firstField = seen.get(normalized);
    if (firstField) {
      duplicateNotices.push(t("composer.autocomplete.duplicateSkipped", {
        address: address.address,
        fieldLabel: recipientFieldLabel(field, t),
        firstFieldLabel: recipientFieldLabel(firstField, t),
      }));
      continue;
    }

    seen.set(normalized, field);
    deduped.push(address);
  }

  return deduped;
}

function parseAddressList(
  field: RecipientFieldName,
  value: string,
  t: TFunction,
): ParsedAddressList {
  const tokens = value
    .split(/[,;\n]+/)
    .map((token) => token.trim())
    .filter(Boolean);
  const addresses: ComposerAddress[] = [];
  const errors: string[] = [];

  for (const token of tokens) {
    const parsed = parseAddressToken(token);
    if (!parsed) {
      errors.push(
        t("composer.errors.invalidAddress", {
          fieldLabel: recipientFieldLabel(field, t),
          value: token,
        }),
      );
      continue;
    }
    addresses.push(parsed);
  }

  return { addresses, errors };
}

function parseAddressToken(token: string): ComposerAddress | null {
  const displayNameMatch = token.match(/^(.*?)<([^<>]+)>$/);
  const name = displayNameMatch?.[1].trim().replace(/^"|"$/g, "") || null;
  const address = (displayNameMatch?.[2] ?? token).trim();

  if (!/^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$/.test(address)) {
    return null;
  }

  return {
    address,
    name,
  };
}

function currentRecipientToken(value: string) {
  const separatorIndex = Math.max(
    value.lastIndexOf(","),
    value.lastIndexOf(";"),
    value.lastIndexOf("\n"),
  );

  return value.slice(separatorIndex + 1).trim();
}

function replaceCurrentRecipientToken(
  value: string,
  suggestion: RecentRecipientSuggestion,
) {
  const separatorIndex = Math.max(
    value.lastIndexOf(","),
    value.lastIndexOf(";"),
    value.lastIndexOf("\n"),
  );
  const prefix = separatorIndex >= 0 ? value.slice(0, separatorIndex + 1) : "";
  const needsSpace = prefix.length > 0 && !prefix.endsWith(" ");
  return `${prefix}${needsSpace ? " " : ""}${formatRecipientSuggestion(suggestion)}, `;
}

function formatRecipientSuggestion(suggestion: RecentRecipientSuggestion) {
  const name = suggestion.name
    ?.replace(/[",;<>]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return name ? `${name} <${suggestion.address}>` : suggestion.address;
}

function formatComposerAddresses(addresses: ComposerAddress[]) {
  return addresses
    .map((address) => {
      const name = address.name
        ?.replace(/[",;<>]/g, " ")
        .replace(/\s+/g, " ")
        .trim();
      return name ? `${name} <${address.address}>` : address.address;
    })
    .join(", ");
}

function normalizeEmailAddress(address: string) {
  return address.trim().toLocaleLowerCase();
}

function recipientFieldLabel(field: RecipientFieldName, t: TFunction) {
  switch (field) {
    case "to":
      return t("composer.fields.to");
    case "cc":
      return t("composer.fields.cc");
    case "bcc":
      return t("composer.fields.bcc");
  }
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function composerHtmlHasContent(html: string) {
  const document = new DOMParser().parseFromString(html, "text/html");
  return Boolean(
    document.body.textContent?.trim() ||
      document.querySelector("img[src], table, pre"),
  );
}

function createDraftId() {
  const random =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now()}_${Math.random().toString(16).slice(2)}`;

  return `draft_${random.replace(/-/g, "_")}`;
}

function fileToDataUrl(file: File) {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

function sendProgressLabel(
  progress: ComposerSendResponse["progress"][number],
  t: TFunction,
) {
  return t(`composer.sendProgress.${progress}`);
}
