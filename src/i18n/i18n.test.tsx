import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, vi } from "vitest";
import App from "../App";
import type { MailShellData } from "../mail/shell/mailShellState";
import {
  createKomorebiI18n,
  detectInitialLanguage,
  formatByteSize,
  formatCount,
  formatDateTime,
  formatRelativeDateTime,
  i18n as appI18n,
  matchSupportedLanguage,
} from ".";
import { en, ptPT } from "./resources";

function flattenedKeys(value: unknown, prefix = ""): string[] {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return [prefix];
  }

  return Object.entries(value).flatMap(([key, child]) =>
    flattenedKeys(child, prefix ? `${prefix}.${key}` : key),
  );
}

describe("i18n", () => {
  afterEach(async () => {
    vi.useRealTimers();
    await act(async () => {
      await appI18n.changeLanguage("en");
    });
  });

  it("detects supported first-launch languages and falls back to English", () => {
    expect(
      detectInitialLanguage({
        language: "en-US",
        languages: ["fr-FR", "pt-PT"],
      }),
    ).toBe("pt-PT");
    expect(
      detectInitialLanguage({
        language: "pt-BR",
        languages: [],
      }),
    ).toBe("pt-PT");
    expect(matchSupportedLanguage("en-GB")).toBe("en");
    expect(
      detectInitialLanguage({
        language: "fr-FR",
        languages: ["fr-FR"],
      }),
    ).toBe("en");
  });

  it("keeps pt-PT resources complete against English", () => {
    expect(flattenedKeys(ptPT).sort()).toEqual(flattenedKeys(en).sort());
  });

  it("renders the first-run shell in pt-PT through the provider", () => {
    const testI18n = createKomorebiI18n("pt-PT");

    render(
      <I18nextProvider i18n={testI18n}>
        <App />
      </I18nextProvider>,
    );

    expect(screen.getByRole("heading", { name: "Komorebi" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Definicoes" })).toBeEnabled();
    expect(screen.getByText("Nenhuma conta configurada.")).toBeVisible();
    expect(
      screen.getByRole("checkbox", { name: "Caixa unificada" }),
    ).toBeDisabled();
  });

  it("applies a persisted language preference when one exists", async () => {
    await act(async () => {
      await appI18n.changeLanguage("en");
    });

    render(<App loadUiPreferences={() => Promise.resolve({ language: "pt-PT" })} />);

    expect(
      await screen.findByRole("button", { name: "Definicoes" }),
    ).toBeEnabled();
    expect(screen.getByText("Nenhuma conta configurada.")).toBeVisible();
  });

  it("formats counts, dates, times, sizes, and statuses for pt-PT UI", () => {
    const testI18n = createKomorebiI18n("pt-PT");
    const localMail: MailShellData = {
      accounts: [
        {
          email: "local@example.com",
          id: "acct_local",
          label: "Local",
          signatureHtml: null,
          status: "syncing",
          unreadCount: 1234567,
        },
      ],
      folders: [
        {
          accountId: "acct_local",
          id: "folder_local_inbox",
          kind: "inbox",
          name: "Inbox",
          totalCount: 1234567,
          unreadCount: 1234567,
        },
      ],
      messages: [
        {
          accountId: "acct_local",
          attachments: [
            {
              contentType: "text/plain",
              displayFilename: "",
              disposition: "attachment",
              id: "att_locale",
              sizeBytes: 1536,
            },
          ],
          bodyHtml: null,
          bodyText: "Corpo local",
          flagged: false,
          folderId: "folder_local_inbox",
          id: "msg_locale",
          preview: "Corpo local",
          receivedAt: "2026-05-22T09:30:00",
          sender: "",
          senderEmail: "",
          subject: "Relatorio",
          unread: true,
        },
      ],
    };

    render(
      <I18nextProvider i18n={testI18n}>
        <App initialData={localMail} />
      </I18nextProvider>,
    );

    expect(screen.getAllByText(/1\s234\s567 por ler/).length).toBeGreaterThan(0);
    expect(screen.getByText("a sincronizar")).toBeVisible();
    expect(screen.getByText("22/05/2026, 09:30")).toBeVisible();
    expect(screen.getByText("Remetente desconhecido")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: /Relatorio/ }));

    const attachments = screen.getByRole("region", { name: "Anexos" });
    expect(within(attachments).getByText("Anexo")).toBeVisible();
    expect(within(attachments).getByText("text/plain - 1,5 KB")).toBeVisible();
  });

  it("renders locale-aware relative timestamps in the shell", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-05-25T12:00:00"));
    const testI18n = createKomorebiI18n("pt-PT");
    const localMail: MailShellData = {
      accounts: [
        {
          email: "local@example.com",
          id: "acct_local",
          label: "Local",
          lastSuccessfulSyncAt: "2026-05-25T08:15:00",
          signatureHtml: null,
          status: "connected",
          unreadCount: 1,
        },
      ],
      folders: [
        {
          accountId: "acct_local",
          id: "folder_local_inbox",
          kind: "inbox",
          name: "Inbox",
          totalCount: 1,
          unreadCount: 1,
        },
      ],
      messages: [
        {
          accountId: "acct_local",
          attachments: [],
          bodyHtml: null,
          bodyText: "Corpo local",
          flagged: false,
          folderId: "folder_local_inbox",
          id: "msg_relative",
          preview: "Corpo local",
          receivedAt: "2026-05-24T09:30:00",
          sender: "Equipa",
          senderEmail: "equipa@example.com",
          subject: "Relativo",
          unread: true,
        },
      ],
    };

    render(
      <I18nextProvider i18n={testI18n}>
        <App initialData={localMail} />
      </I18nextProvider>,
    );

    expect(screen.getByText("Ontem, 09:30")).toBeVisible();
    expect(
      screen.getByText("Ultima sincronizacao: Hoje, 08:15"),
    ).toBeVisible();
  });

  it("exposes reusable locale formatters with English fallback", () => {
    const testI18n = createKomorebiI18n("en");
    const ptI18n = createKomorebiI18n("pt-PT");
    const referenceDate = new Date("2026-05-25T12:00:00");

    expect(formatCount(1234567, "en")).toBe("1,234,567");
    expect(formatCount(1234567, "pt-PT")).toMatch(/^1\s234\s567$/);
    expect(formatDateTime("2026-05-22T09:30:00", "pt-PT")).toBe(
      "22/05/2026, 09:30",
    );
    expect(
      formatRelativeDateTime(
        "2026-05-25T09:30:00",
        "en",
        testI18n.t,
        referenceDate,
      ),
    ).toBe("Today, 9:30 AM");
    expect(
      formatRelativeDateTime(
        "2026-05-24T09:30:00",
        "pt-PT",
        ptI18n.t,
        referenceDate,
      ),
    ).toBe("Ontem, 09:30");
    expect(
      formatRelativeDateTime(
        "2026-05-26T09:30:00",
        "pt-PT",
        ptI18n.t,
        referenceDate,
      ),
    ).toBe("Amanha, 09:30");
    expect(
      formatRelativeDateTime(
        "2026-05-22T09:30:00",
        "pt-PT",
        ptI18n.t,
        referenceDate,
      ),
    ).toBe("22/05/2026, 09:30");
    expect(formatDateTime("Yesterday", "pt-PT")).toBe("Yesterday");
    expect(formatByteSize(1536, "pt-PT", testI18n.t)).toBe("1,5 KB");
    expect(formatByteSize(0, "fr-FR", testI18n.t)).toBe("0 B");
  });
});
