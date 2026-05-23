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

  it("rewrites safe links for delegated opening and blocks unsafe links", () => {
    const sanitized = sanitizeEmailHtml(
      '<a href="https://example.com/agenda">Agenda</a><a href="javascript:alert(1)">Bad</a><a href="/relative">Relative</a>',
    );
    const document = parseSanitized(sanitized.html);
    const safeLink = document.querySelector<HTMLAnchorElement>(
      'a[data-komorebi-url="https://example.com/agenda"]',
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
    ]);
    expect(safeLink?.getAttribute("href")).toBe("#");
    expect(safeLink?.getAttribute("rel")).toBe("noreferrer noopener nofollow");
    expect(blockedLink).toBeDefined();
    expect(blockedLink?.getAttribute("href")).toBeNull();
    expect(blockedLink?.getAttribute("data-komorebi-url")).toBeNull();
    expect(relativeLink).toBeDefined();
    expect(relativeLink?.getAttribute("href")).toBeNull();
    expect(relativeLink?.getAttribute("data-komorebi-url")).toBeNull();
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
