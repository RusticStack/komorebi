import DOMPurify, { type Config } from "dompurify";

export type SafeEmailLink = {
  id: string;
  url: string;
  label: string;
};

export type SanitizedEmailHtml = {
  html: string;
  plainText: string;
  links: SafeEmailLink[];
  remoteContentBlocked: boolean;
  blockedRemoteResourceCount: number;
};

const EMAIL_SANITIZER_CONFIG: Config = {
  USE_PROFILES: { html: true },
  SANITIZE_NAMED_PROPS: true,
  ALLOW_ARIA_ATTR: true,
  ALLOW_DATA_ATTR: true,
  FORBID_TAGS: [
    "base",
    "button",
    "canvas",
    "embed",
    "form",
    "iframe",
    "input",
    "link",
    "math",
    "meta",
    "object",
    "option",
    "script",
    "select",
    "style",
    "svg",
    "textarea",
  ],
  FORBID_ATTR: [
    "background",
    "form",
    "formaction",
    "poster",
    "src",
    "srcdoc",
    "srcset",
    "style",
    "xlink:href",
  ],
};

const REMOTE_RESOURCE_PATTERN =
  /\b(?:background|poster|src|srcset)\s*=\s*["']?\s*(?:https?:|\/\/|cid:|data:)|(?:@import\s+(?:url\()?["']?\s*(?:https?:|\/\/|cid:|data:)|url\s*\(\s*["']?\s*(?:https?:|\/\/|cid:|data:))/gi;
const RESERVED_KOMOREBI_DATA_ATTRIBUTE = /^data-komorebi-/i;

export function sanitizeEmailHtml(input: string): SanitizedEmailHtml {
  const blockedRemoteResourceCount = countBlockedResourceReferences(input);

  if (!DOMPurify.isSupported) {
    return {
      html: "",
      plainText: input,
      links: [],
      remoteContentBlocked: blockedRemoteResourceCount > 0,
      blockedRemoteResourceCount,
    };
  }

  const sanitized = DOMPurify.sanitize(input, EMAIL_SANITIZER_CONFIG) as string;
  const document = new DOMParser().parseFromString(sanitized, "text/html");
  stripReservedKomorebiDataAttributes(document);
  const links = rewriteLinks(document);

  const html = DOMPurify.sanitize(
    document.body.innerHTML,
    EMAIL_SANITIZER_CONFIG,
  ) as string;

  return {
    html,
    plainText: document.body.textContent?.trim() ?? "",
    links,
    remoteContentBlocked: blockedRemoteResourceCount > 0,
    blockedRemoteResourceCount,
  };
}

function stripReservedKomorebiDataAttributes(document: Document) {
  for (const element of document.querySelectorAll("*")) {
    for (const attribute of Array.from(element.attributes)) {
      if (RESERVED_KOMOREBI_DATA_ATTRIBUTE.test(attribute.name)) {
        element.removeAttribute(attribute.name);
      }
    }
  }
}

export function isSafeOpenableUrl(value: string): URL | null {
  let parsed: URL;

  try {
    parsed = new URL(value);
  } catch {
    return null;
  }

  if (!["http:", "https:", "mailto:"].includes(parsed.protocol)) {
    return null;
  }

  if (parsed.username || parsed.password) {
    return null;
  }

  if (parsed.protocol !== "mailto:" && !parsed.hostname) {
    return null;
  }

  return parsed;
}

function rewriteLinks(document: Document): SafeEmailLink[] {
  const links: SafeEmailLink[] = [];

  for (const anchor of document.querySelectorAll("a[href]")) {
    const href = anchor.getAttribute("href");
    const parsed = href ? isSafeOpenableUrl(href) : null;

    anchor.setAttribute("rel", "noreferrer noopener nofollow");
    anchor.removeAttribute("target");

    if (!parsed) {
      anchor.removeAttribute("href");
      anchor.setAttribute("data-komorebi-link-blocked", "true");
      continue;
    }

    const link = {
      id: `email-link-${links.length + 1}`,
      url: parsed.href,
      label: anchor.textContent?.trim() || parsed.href,
    };

    anchor.setAttribute("href", "#");
    anchor.setAttribute("data-komorebi-link-id", link.id);
    anchor.setAttribute("data-komorebi-url", link.url);
    links.push(link);
  }

  return links;
}

function countBlockedResourceReferences(input: string): number {
  return Array.from(input.matchAll(REMOTE_RESOURCE_PATTERN)).length;
}
