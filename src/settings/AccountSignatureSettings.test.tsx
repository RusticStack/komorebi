import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AccountSignatureSettings } from "./AccountSignatureSettings";

describe("AccountSignatureSettings", () => {
  it("renders the accountless settings state without seeded accounts", () => {
    render(
      <AccountSignatureSettings accounts={[]} onSaveSignature={() => undefined} />,
    );

    expect(screen.getByRole("region", { name: "Settings" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "Signatures" })).toBeVisible();
    expect(screen.getByText("No accounts configured.")).toBeVisible();
  });

  it("sanitizes the edited signature before saving per account", () => {
    const onSaveSignature = vi.fn();
    render(
      <AccountSignatureSettings
        accounts={[
          {
            id: "acct_1",
            label: "Personal",
            email: "sender@example.com",
            signatureHtml: null,
          },
        ]}
        onSaveSignature={onSaveSignature}
      />,
    );

    fireEvent.change(screen.getByLabelText("Signature HTML"), {
      target: {
        value:
          '<p onclick="alert(1)">Regards<br><a href="https://example.com">Sender</a><img src="https://tracker.example/pixel.gif"></p>',
      },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save signature" }));

    expect(onSaveSignature).toHaveBeenCalledTimes(1);
    const [accountId, signatureHtml] = onSaveSignature.mock.calls[0] as [
      string,
      string,
    ];
    expect(accountId).toBe("acct_1");
    expect(signatureHtml).toContain("Regards");
    expect(signatureHtml).toContain("https://example.com/");
    expect(signatureHtml).not.toContain("onclick");
    expect(signatureHtml).not.toContain("<img");
    expect(signatureHtml).not.toContain("tracker.example");
  });

  it("loads each account signature and can intentionally clear it", () => {
    const onSaveSignature = vi.fn();
    render(
      <AccountSignatureSettings
        accounts={[
          {
            id: "acct_1",
            label: "Personal",
            email: "sender@example.com",
            signatureHtml: "<p>Personal signature</p>",
          },
          {
            id: "acct_2",
            label: "Work",
            email: "work@example.com",
            signatureHtml: "<p>Work signature</p>",
          },
        ]}
        onSaveSignature={onSaveSignature}
      />,
    );

    expect(screen.getByLabelText("Signature HTML")).toHaveValue(
      "<p>Personal signature</p>",
    );

    fireEvent.change(screen.getByLabelText("Account"), {
      target: { value: "acct_2" },
    });
    expect(screen.getByLabelText("Signature HTML")).toHaveValue(
      "<p>Work signature</p>",
    );

    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    fireEvent.click(screen.getByRole("button", { name: "Save signature" }));

    expect(onSaveSignature).toHaveBeenCalledWith("acct_2", null);
  });

  it("validates and creates compatible generic IMAP/SMTP accounts through typed callbacks", async () => {
    const onCreateAccount = vi.fn().mockResolvedValue({
      account: {
        authKind: "app_password",
        authStatus: "connected",
        displayEmail: "person@example.com",
        displayName: "Personal",
        id: "acct_generic",
        providerKind: "generic_imap_smtp",
        username: "person@example.com",
      },
      credentialStored: true,
    });
    const onTestAccountConnection = vi.fn().mockResolvedValue({
      authKind: "app_password",
      providerKind: "generic_imap_smtp",
      status: "configuration-ready",
    });
    const onAccountChanged = vi.fn();

    render(
      <AccountSignatureSettings
        accounts={[]}
        onAccountChanged={onAccountChanged}
        onCreateAccount={onCreateAccount}
        onSaveSignature={() => undefined}
        onTestAccountConnection={onTestAccountConnection}
      />,
    );

    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "person@example.com" },
    });
    fireEvent.change(screen.getByLabelText("Display name"), {
      target: { value: "Personal" },
    });
    fireEvent.change(screen.getByLabelText("Username"), {
      target: { value: "person@example.com" },
    });
    fireEvent.change(screen.getByLabelText("IMAP host"), {
      target: { value: "imap.example.com" },
    });
    fireEvent.change(screen.getByLabelText("SMTP host"), {
      target: { value: "smtp.example.com" },
    });
    fireEvent.change(screen.getByLabelText("Password or app password"), {
      target: { value: "app-password" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Test setup" }));
    expect(await screen.findByText("Configuration validated.")).toBeVisible();
    expect(onTestAccountConnection).toHaveBeenCalledWith({
      authKind: "app_password",
      displayEmail: "person@example.com",
      displayName: "Personal",
      imapHost: "imap.example.com",
      imapPort: 993,
      imapSecurity: "tls",
      providerKind: "generic_imap_smtp",
      secret: "app-password",
      smtpHost: "smtp.example.com",
      smtpPort: 587,
      smtpSecurity: "starttls",
      username: "person@example.com",
    });

    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(onCreateAccount).toHaveBeenCalledTimes(1));
    expect(onCreateAccount).toHaveBeenCalledWith({
      authKind: "app_password",
      displayEmail: "person@example.com",
      displayName: "Personal",
      imapHost: "imap.example.com",
      imapPort: 993,
      imapSecurity: "tls",
      providerKind: "generic_imap_smtp",
      secret: "app-password",
      smtpHost: "smtp.example.com",
      smtpPort: 587,
      smtpSecurity: "starttls",
      username: "person@example.com",
    });
    expect(onAccountChanged).toHaveBeenCalledTimes(1);
  });

  it("forces major providers through OAuth setup without generic server fields", async () => {
    const onCreateAccount = vi.fn().mockResolvedValue({
      account: {
        authKind: "oauth2",
        authStatus: "connected",
        displayEmail: "person@gmail.com",
        displayName: null,
        id: "acct_gmail",
        providerKind: "gmail",
        username: "person@gmail.com",
      },
      credentialStored: true,
    });

    render(
      <AccountSignatureSettings
        accounts={[]}
        onCreateAccount={onCreateAccount}
        onSaveSignature={() => undefined}
      />,
    );

    fireEvent.change(screen.getByLabelText("Provider"), {
      target: { value: "gmail" },
    });
    expect(screen.queryByLabelText("Authentication")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("IMAP host")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Email address"), {
      target: { value: "person@gmail.com" },
    });
    fireEvent.change(screen.getByLabelText("Username"), {
      target: { value: "person@gmail.com" },
    });
    fireEvent.change(screen.getByLabelText("OAuth refresh token"), {
      target: { value: "refresh-token" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(onCreateAccount).toHaveBeenCalledTimes(1));
    expect(onCreateAccount).toHaveBeenCalledWith({
      authKind: "oauth2",
      displayEmail: "person@gmail.com",
      displayName: null,
      imapHost: null,
      imapPort: null,
      imapSecurity: null,
      providerKind: "gmail",
      secret: "refresh-token",
      smtpHost: null,
      smtpPort: null,
      smtpSecurity: null,
      username: "person@gmail.com",
    });
  });

  it("reauthenticates, checks redacted status, and removes the selected account", async () => {
    const onGetAccountAuthStatus = vi.fn().mockResolvedValue({
      accountId: "acct_1",
      authKind: "app_password",
      authStatus: "connected",
      credentialPresent: true,
      providerKind: "generic_imap_smtp",
    });
    const onReauthAccount = vi.fn().mockResolvedValue({
      account: {
        authKind: "app_password",
        authStatus: "connected",
        displayEmail: "sender@example.com",
        displayName: "Personal",
        id: "acct_1",
        providerKind: "generic_imap_smtp",
        username: "sender@example.com",
      },
      credentialStored: true,
    });
    const onRemoveAccount = vi.fn().mockResolvedValue({
      accountId: "acct_1",
      credentialsDeleted: true,
      localCacheDeleted: true,
      metadataRemoved: true,
    });

    render(
      <AccountSignatureSettings
        accounts={[
          {
            email: "sender@example.com",
            id: "acct_1",
            label: "Personal",
            signatureHtml: null,
          },
        ]}
        onGetAccountAuthStatus={onGetAccountAuthStatus}
        onReauthAccount={onReauthAccount}
        onRemoveAccount={onRemoveAccount}
        onSaveSignature={() => undefined}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Check status" }));
    expect(
      await screen.findByText("Auth status: connected; credential present."),
    ).toBeVisible();
    expect(onGetAccountAuthStatus).toHaveBeenCalledWith({
      accountId: "acct_1",
    });

    fireEvent.change(screen.getByLabelText("Replacement secret"), {
      target: { value: "replacement-password" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Reauthenticate" }));

    await waitFor(() => expect(onReauthAccount).toHaveBeenCalledWith({
      accountId: "acct_1",
      secret: "replacement-password",
    }));

    fireEvent.click(screen.getByRole("button", { name: "Remove account" }));
    await waitFor(() =>
      expect(onRemoveAccount).toHaveBeenCalledWith({
        accountId: "acct_1",
        deleteLocalCache: true,
      }),
    );
  });
});
