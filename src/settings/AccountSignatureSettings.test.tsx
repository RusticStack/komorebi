import { fireEvent, render, screen } from "@testing-library/react";
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
});
