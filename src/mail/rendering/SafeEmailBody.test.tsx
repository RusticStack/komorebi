import { fireEvent, render, screen } from "@testing-library/react";
import { SafeEmailBody } from "./SafeEmailBody";
import { sanitizeEmailHtml } from "./emailHtml";
import { sanitizerBypassFixtures, xssFixtures } from "./xssFixtures";

const dangerousSelector = [
  "script",
  "iframe",
  "object",
  "embed",
  "svg",
  "math",
  "style",
  "meta",
  "link",
  "form",
  "input",
].join(",");
const dangerousAttributePattern =
  /^(background|form|formaction|poster|src|srcdoc|srcset|style|xlink:href)$/i;
const dangerousValuePattern = /(?:javascript:|data:text\/html|data:image\/svg)/i;

function parseSanitized(html: string) {
  return new DOMParser().parseFromString(html, "text/html");
}

function expectSanitizedDocumentIsSafe(document: Document) {
  expect(document.querySelector(dangerousSelector)).toBeNull();

  for (const element of document.querySelectorAll("*")) {
    for (const attribute of Array.from(element.attributes)) {
      expect(attribute.name).not.toMatch(/^on/i);
      expect(attribute.name).not.toMatch(dangerousAttributePattern);
      expect(attribute.value).not.toMatch(dangerousValuePattern);
    }
  }
}

describe("sanitizeEmailHtml", () => {
  it.each(xssFixtures)("sanitizes XSS fixture: $name", ({ html }) => {
    const sanitized = sanitizeEmailHtml(html);
    const document = parseSanitized(sanitized.html);

    expectSanitizedDocumentIsSafe(document);
  });

  it.each(sanitizerBypassFixtures)(
    "sanitizes bypass fixture: $category / $name",
    ({ html, remoteResourceCount }) => {
      const sanitized = sanitizeEmailHtml(html);
      const document = parseSanitized(sanitized.html);

      expectSanitizedDocumentIsSafe(document);
      expect(sanitized.html).not.toMatch(/<script|<style|<svg|<math/i);
      expect(sanitized.html).not.toMatch(/\son\w+\s*=/i);
      expect(sanitized.html).not.toMatch(/javascript:|data:text\/html/i);

      if (remoteResourceCount !== undefined) {
        expect(sanitized.remoteContentBlocked).toBe(true);
        expect(sanitized.blockedRemoteResourceCount).toBe(remoteResourceCount);
      }
    },
  );

  it("keeps sanitized output safe after browser insertion and serialization", () => {
    for (const fixture of sanitizerBypassFixtures.filter(
      ({ category }) => category === "post-sanitization-mutation",
    )) {
      const sanitized = sanitizeEmailHtml(fixture.html);
      const host = document.createElement("div");
      host.innerHTML = sanitized.html;

      expectSanitizedDocumentIsSafe(parseSanitized(host.innerHTML));
      expectSanitizedDocumentIsSafe(parseSanitized(sanitizeEmailHtml(host.innerHTML).html));
    }
  });

  it("strips attacker-supplied Komorebi link metadata before rewriting links", () => {
    const sanitized = sanitizeEmailHtml(
      '<a data-komorebi-url="https://evil.example" data-komorebi-link-id="email-link-1">Trap</a><a href="https://example.com/safe">Safe</a>',
    );
    const document = parseSanitized(sanitized.html);
    const trapLink = Array.from(document.querySelectorAll("a")).find(
      (anchor) => anchor.textContent === "Trap",
    );
    const safeLink = Array.from(document.querySelectorAll("a")).find(
      (anchor) => anchor.textContent === "Safe",
    );

    expect(sanitized.links).toEqual([
      {
        id: "email-link-1",
        label: "Safe",
        url: "https://example.com/safe",
      },
    ]);
    expect(trapLink?.getAttribute("data-komorebi-url")).toBeNull();
    expect(trapLink?.getAttribute("data-komorebi-link-id")).toBeNull();
    expect(safeLink?.getAttribute("data-komorebi-url")).toBe(
      "https://example.com/safe",
    );
  });

  it("blocks remote resources by removing loading attributes", () => {
    const sanitized = sanitizeEmailHtml(
      '<p>Hi</p><img src="https://tracker.example/open.png" srcset="https://tracker.example/2x.png 2x" alt="tracking pixel">',
    );
    const document = parseSanitized(sanitized.html);
    const image = document.querySelector("img");

    expect(sanitized.remoteContentBlocked).toBe(true);
    expect(sanitized.blockedRemoteResourceCount).toBe(2);
    expect(image).not.toBeNull();
    expect(image?.getAttribute("src")).toBeNull();
    expect(image?.getAttribute("srcset")).toBeNull();
    expect(image?.getAttribute("alt")).toBe("tracking pixel");
  });

  it("allows only validated HTTPS image resources after explicit consent", () => {
    const sanitized = sanitizeEmailHtml(
      [
        '<img src="https://tracker.example/open.png" alt="tracking pixel">',
        '<img src="http://tracker.example/insecure.png" alt="insecure pixel">',
        '<img src="https://user:pass@tracker.example/credential.png" alt="credential pixel">',
        '<img src="https://127.0.0.1/internal.png" alt="internal pixel">',
        '<img src="data:image/png;base64,AAAA" alt="data pixel">',
        '<img src="cid:logo" alt="cid pixel">',
      ].join(""),
      "allowed",
    );
    const document = parseSanitized(sanitized.html);
    const allowedImage = document.querySelector<HTMLImageElement>(
      'img[alt="tracking pixel"]',
    );
    const blockedImages = [
      "insecure pixel",
      "credential pixel",
      "internal pixel",
      "data pixel",
      "cid pixel",
    ].map((alt) => document.querySelector<HTMLImageElement>(`img[alt="${alt}"]`));

    expect(allowedImage).not.toBeNull();
    expect(allowedImage?.getAttribute("src")).toBe(
      "https://tracker.example/open.png",
    );
    expect(allowedImage?.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(sanitized.allowedRemoteResourceCount).toBe(1);
    expect(sanitized.blockedRemoteResourceCount).toBe(5);
    expect(sanitized.remoteContentBlocked).toBe(true);
    for (const image of blockedImages) {
      expect(image).not.toBeNull();
      expect(image?.getAttribute("src")).toBeNull();
    }
  });

  it("reports fully allowed remote image content without the blocked status", () => {
    const sanitized = sanitizeEmailHtml(
      '<p>Hi</p><img src="https://cdn.example/logo.png" alt="logo">',
      "allowed",
    );
    const document = parseSanitized(sanitized.html);

    expect(document.querySelector("img")?.getAttribute("src")).toBe(
      "https://cdn.example/logo.png",
    );
    expect(sanitized.allowedRemoteResourceCount).toBe(1);
    expect(sanitized.blockedRemoteResourceCount).toBe(0);
    expect(sanitized.remoteContentBlocked).toBe(false);
  });

  it("keeps unsafe links delegated or stripped even when remote images are allowed", () => {
    const sanitized = sanitizeEmailHtml(
      '<img src="https://cdn.example/logo.png" alt="logo"><a href="javascript:alert(1)">bad</a><a href="https://example.com/safe">safe</a>',
      "allowed",
    );
    const document = parseSanitized(sanitized.html);

    expect(document.querySelector("img")?.getAttribute("src")).toBe(
      "https://cdn.example/logo.png",
    );
    expect(
      document.querySelector<HTMLAnchorElement>('a[data-komorebi-url]')?.dataset
        .komorebiUrl,
    ).toBe("https://example.com/safe");
    expect(
      Array.from(document.querySelectorAll("a")).find(
        (anchor) => anchor.textContent === "bad",
      )?.getAttribute("href"),
    ).toBeNull();
  });

  it("rewrites safe links for delegated opening and blocks unsafe links", () => {
    const sanitized = sanitizeEmailHtml(
      [
        '<a href="https://example.com/agenda">Agenda</a>',
        '<a href="http://example.com/read">Read</a>',
        '<a href="mailto:person@example.com?subject=Hello">Mail</a>',
        '<a href="javascript:alert(1)">Bad</a>',
        '<a href="/relative">Relative</a>',
        '<a href="https://user:pass@example.com">Credentialed</a>',
        '<a href="https://localhost/internal">Localhost</a>',
        '<a href="mailto:person@example.com?body=hidden">Mail body</a>',
      ].join(""),
    );
    const document = parseSanitized(sanitized.html);
    const safeLink = document.querySelector<HTMLAnchorElement>(
      'a[data-komorebi-url="https://example.com/agenda"]',
    );
    const mailtoLink = document.querySelector<HTMLAnchorElement>(
      'a[data-komorebi-url="mailto:person@example.com?subject=Hello"]',
    );
    const blockedLink = Array.from(document.querySelectorAll("a")).find(
      (anchor) => anchor.textContent === "Bad",
    );
    const relativeLink = Array.from(document.querySelectorAll("a")).find(
      (anchor) => anchor.textContent === "Relative",
    );

    expect(sanitized.links).toEqual([
      {
        id: "email-link-1",
        url: "https://example.com/agenda",
        label: "Agenda",
      },
      {
        id: "email-link-2",
        url: "http://example.com/read",
        label: "Read",
      },
      {
        id: "email-link-3",
        url: "mailto:person@example.com?subject=Hello",
        label: "Mail",
      },
    ]);
    expect(safeLink?.getAttribute("href")).toBe("#");
    expect(safeLink?.getAttribute("rel")).toBe("noreferrer noopener nofollow");
    expect(mailtoLink?.getAttribute("href")).toBe("#");
    expect(blockedLink).toBeDefined();
    expect(blockedLink?.getAttribute("href")).toBeNull();
    expect(blockedLink?.getAttribute("data-komorebi-url")).toBeNull();
    expect(relativeLink).toBeDefined();
    expect(relativeLink?.getAttribute("href")).toBeNull();
    expect(relativeLink?.getAttribute("data-komorebi-url")).toBeNull();
    for (const label of ["Credentialed", "Localhost", "Mail body"]) {
      const blocked = Array.from(document.querySelectorAll("a")).find(
        (anchor) => anchor.textContent === label,
      );
      expect(blocked?.getAttribute("href")).toBeNull();
      expect(blocked?.getAttribute("data-komorebi-url")).toBeNull();
    }
  });

  it("counts remote resources hidden in CSS before removing styles", () => {
    const sanitized = sanitizeEmailHtml(
      '<style>@import url(https://tracker.example/open.css)</style><p style="background-image: url(https://tracker.example/open.png)">Hi</p>',
    );
    const document = parseSanitized(sanitized.html);

    expect(sanitized.remoteContentBlocked).toBe(true);
    expect(sanitized.blockedRemoteResourceCount).toBe(2);
    expect(document.querySelector("style")).toBeNull();
    expect(document.querySelector("p")?.getAttribute("style")).toBeNull();
  });
});

describe("SafeEmailBody", () => {
  it("renders sanitized HTML and delegates safe link opening", () => {
    const onOpenExternalLink = vi.fn();

    render(
      <SafeEmailBody
        bodyHtml='<p>Read <a href="https://example.com/agenda">agenda</a>.</p>'
        bodyText="Read agenda."
        onOpenExternalLink={onOpenExternalLink}
      />,
    );

    const link = screen.getByRole("link", { name: "agenda" });

    expect(link).toHaveAttribute("href", "#");

    fireEvent.click(link);

    expect(onOpenExternalLink).toHaveBeenCalledWith({
      id: "email-link-1",
      url: "https://example.com/agenda",
      label: "agenda",
    });
  });

  it("renders plaintext fallback without interpreting markup", () => {
    const { container } = render(
      <SafeEmailBody bodyText="<script>alert(1)</script>\nPlain body" />,
    );

    expect(container.querySelector("script")).toBeNull();
    expect(screen.getByText(/<script>alert\(1\)<\/script>/)).toBeVisible();
    expect(screen.getByText(/Plain body/)).toBeVisible();
  });

  it("shows a blocked remote content status", () => {
    render(
      <SafeEmailBody bodyHtml='<img src="https://tracker.example/pixel.png" alt="pixel">' />,
    );

    expect(screen.getByRole("status")).toHaveTextContent(
      "Remote content blocked",
    );
  });

  it("renders consented remote images without showing the blocked status", () => {
    const { container } = render(
      <SafeEmailBody
        allowRemoteContent
        bodyHtml='<img src="https://cdn.example/logo.png" alt="logo">'
      />,
    );

    expect(container.querySelector("img")?.getAttribute("src")).toBe(
      "https://cdn.example/logo.png",
    );
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("does not delegate clicks from attacker-supplied post-sanitization link metadata", () => {
    const onOpenExternalLink = vi.fn();

    render(
      <SafeEmailBody
        bodyHtml='<a data-komorebi-url="https://evil.example" data-komorebi-link-id="email-link-1">Trap</a><a href="https://example.com/safe">Safe</a>'
        onOpenExternalLink={onOpenExternalLink}
      />,
    );

    fireEvent.click(screen.getByText("Trap"));

    expect(onOpenExternalLink).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("link", { name: "Safe" }));

    expect(onOpenExternalLink).toHaveBeenCalledWith({
      id: "email-link-1",
      label: "Safe",
      url: "https://example.com/safe",
    });
  });
});
