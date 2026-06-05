import type { TFunction } from "i18next";
import type { useLocaleFormatters } from "../../i18n";
import type {
  AccountManagementView,
  MailAccount,
  MailShellData,
} from "./mailShellState";

export function accountStatusLabel(
  status: MailShellData["accounts"][number]["status"],
  t: TFunction,
) {
  return t(`mail.account.status.${status}`);
}

export function accountNeedsAttention(account: MailAccount) {
  return (
    account.status === "reconnect-needed" ||
    account.status === "error" ||
    account.statusAction === "reauthenticate"
  );
}

export function lastSyncLabel(
  account: MailAccount,
  localeFormat: ReturnType<typeof useLocaleFormatters>,
  t: TFunction,
) {
  return t("mail.account.lastSync", {
    time: lastSyncValue(account, localeFormat, t),
  });
}

export function lastSyncValue(
  account: MailAccount,
  localeFormat: ReturnType<typeof useLocaleFormatters>,
  t: TFunction,
) {
  return account.lastSuccessfulSyncAt
    ? localeFormat.relativeDateTime(account.lastSuccessfulSyncAt)
    : t("mail.account.lastSyncNever");
}

export function statusMessageFallback(account: MailAccount, t: TFunction) {
  if (account.status === "reconnect-needed") {
    return t("mail.account.statusDrawer.reconnectNeededMessage");
  }
  if (account.status === "error") {
    return t("mail.account.statusDrawer.errorMessage");
  }
  if (account.status === "offline") {
    return t("mail.account.statusDrawer.offlineMessage");
  }
  if (account.status === "syncing") {
    return t("mail.account.statusDrawer.syncingMessage");
  }

  return t("mail.account.statusDrawer.idleMessage");
}

export function formatAccountError(error: unknown) {
  if (error instanceof Error) {
    return error.message;
  }

  if (
    error &&
    typeof error === "object" &&
    "reason" in error &&
    typeof error.reason === "string"
  ) {
    return error.reason;
  }

  return String(error);
}

export function authStatusForManagement(
  status: MailShellData["accounts"][number]["status"],
): AccountManagementView["authStatus"] {
  switch (status) {
    case "connected":
    case "idle":
    case "syncing":
    case "error":
      return "connected";
    case "reconnect-needed":
      return "reconnect_needed";
    default:
      return "disconnected";
  }
}
