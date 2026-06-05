import type { TFunction } from "i18next";
import type { MailMessage } from "./mailShellState";

export function messageSenderLabel(
  message: Pick<MailMessage, "sender" | "senderEmail">,
  t: TFunction,
) {
  return (
    message.sender.trim() ||
    message.senderEmail.trim() ||
    t("mail.messages.unknownSender")
  );
}
