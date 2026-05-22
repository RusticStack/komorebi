import {
  insertSignatureIntoComposerHtml,
  sanitizeSignatureHtml,
  SIGNATURE_HTML_BLOCK_START,
  signatureHtmlToPlainText,
} from "./signatureHtml";

describe("signatureHtml", () => {
  it("sanitizes account signatures before storage", () => {
    const sanitized = sanitizeSignatureHtml(
      `<p onclick="alert(1)">Regards<br><a href="https://example.com/profile">Sender</a><img src="https://tracker.example/pixel.gif"></p>`,
    );

    expect(sanitized.html).toContain("Regards");
    expect(sanitized.html).toContain("https://example.com/profile");
    expect(sanitized.html).toContain("rel=\"noreferrer noopener nofollow\"");
    expect(sanitized.html).not.toContain("onclick");
    expect(sanitized.html).not.toContain("<img");
    expect(sanitized.html).not.toContain("tracker.example");
  });

  it("strips unsafe signature links", () => {
    const sanitized = sanitizeSignatureHtml(
      `<a href="javascript:alert(1)">bad</a><a href="https://user:pass@example.com">creds</a>`,
    );

    expect(sanitized.html).not.toContain("javascript:");
    expect(sanitized.html).not.toContain("user:pass");
    expect(sanitized.html).toContain(">bad</a>");
    expect(sanitized.html).toContain(">creds</a>");
  });

  it("inserts and replaces a single predictable composer signature block", () => {
    const first = insertSignatureIntoComposerHtml(
      "<p>Hello</p>",
      "<p>Regards,<br><strong>Sender</strong></p>",
    );
    const second = insertSignatureIntoComposerHtml(
      first,
      "<p>Updated,<br><strong>Sender</strong></p>",
    );

    expect(first).toContain(SIGNATURE_HTML_BLOCK_START);
    expect(first).toContain("komorebi-account-signature");
    expect(second.match(/komorebi-signature:start/g)).toHaveLength(1);
    expect(second).toContain("Updated");
    expect(second).not.toContain("Regards");
  });

  it("removes the managed composer block when the account signature is cleared", () => {
    const signed = insertSignatureIntoComposerHtml(
      "<p>Hello</p>",
      "<p>Sender</p>",
    );

    expect(insertSignatureIntoComposerHtml(signed, null)).toBe("<p>Hello</p>");
  });

  it("derives a plain-text preview from the sanitized signature", () => {
    expect(
      signatureHtmlToPlainText(
        `<p>Regards,<br><a href="mailto:sender@example.com">Sender</a></p>`,
      ),
    ).toBe("Regards,Sender");
  });
});
