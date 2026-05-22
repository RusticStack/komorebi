import { fireEvent, render, screen } from "@testing-library/react";
import { SafeEmailBody } from "./SafeEmailBody";
import { sanitizeEmailHtml } from "./emailHtml";
import { xssFixtures } from "./xssFixtures";

function parseSanitized(html: string) {
  return new DOMParser().parseFromString(html, "text/html");
}

describe("sanitizeEmailHtml", () => {
  it.each(xssFixtures)("sanitizes XSS fixture: $name", ({ html }) => {
    const sanitized = sanitizeEmailHtml(html);
    const document = parseSanitized(sanitized.html);

    expect(
      document.querySelector(
        "script, iframe, object, embed, svg, math, style, meta, link, form, input",
      ),
    ).toBeNull();

    for (const element of document.querySelectorAll("*")) {
      for (const attribute of Array.from(element.attributes)) {
        expect(attribute.name).not.toMatch(/^on/i);
        expect(attribute.name).not.toMatch(
          /^(background|formaction|poster|src|srcdoc|srcset|style|xlink:href)$/i,
        );
        expect(attribute.value).not.toMatch(/javascript:|data:text\/html/i);
      }
    }
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
});
