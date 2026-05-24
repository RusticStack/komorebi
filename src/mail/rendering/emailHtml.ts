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
  allowedRemoteResourceCount: number;
};

export type EmailRemoteContentMode = "blocked" | "allowed";

const BLOCKED_REMOTE_CONTENT_SANITIZER_CONFIG: Config = {
  USE_PROFILES: { html: true },
  SANITIZE_NAMED_PROPS: true,
  ALLOW_ARIA_ATTR: true,
  ALLOW_DATA_ATTR: true,
  ADD_ATTR: ["loading", "referrerpolicy"],
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

const ALLOWED_REMOTE_CONTENT_SANITIZER_CONFIG: Config = {
  ...BLOCKED_REMOTE_CONTENT_SANITIZER_CONFIG,
  FORBID_ATTR: BLOCKED_REMOTE_CONTENT_SANITIZER_CONFIG.FORBID_ATTR?.filter(
    (attribute) => attribute !== "src",
  ),
};

const REMOTE_RESOURCE_PATTERN =
  /\b(?:background|poster|src|srcset)\s*=\s*["']?\s*(?:https?:|\/\/|cid:|data:)|(?:@import\s+(?:url\()?["']?\s*(?:https?:|\/\/|cid:|data:)|url\s*\(\s*["']?\s*(?:https?:|\/\/|cid:|data:))/gi;
const RESERVED_KOMOREBI_DATA_ATTRIBUTE = /^data-komorebi-/i;
const LOCAL_OR_PRIVATE_HOST_PATTERN =
  /^(?:localhost|.*\.localhost|127\.|0\.|10\.|192\.168\.|169\.254\.|172\.(?:1[6-9]|2\d|3[01])\.|::1|\[::1\]|fc[0-9a-f]{2}:|fd[0-9a-f]{2}:|fe80:)/i;
const MAILTO_ALLOWED_FIELDS = new Set(["bcc", "cc", "subject", "to"]);

export function sanitizeEmailHtml(
  input: string,
  remoteContentMode: EmailRemoteContentMode = "blocked",
): SanitizedEmailHtml {
  const remoteResourceReferenceCount = countRemoteResourceReferences(input);
  const sanitizerConfig =
    remoteContentMode === "allowed"
      ? ALLOWED_REMOTE_CONTENT_SANITIZER_CONFIG
      : BLOCKED_REMOTE_CONTENT_SANITIZER_CONFIG;

  if (!DOMPurify.isSupported) {
    return {
      html: "",
      plainText: input,
      links: [],
      remoteContentBlocked: remoteResourceReferenceCount > 0,
      blockedRemoteResourceCount: remoteResourceReferenceCount,
      allowedRemoteResourceCount: 0,
    };
  }

  const sanitized = DOMPurify.sanitize(input, sanitizerConfig) as string;
  const document = new DOMParser().parseFromString(sanitized, "text/html");
  stripReservedKomorebiDataAttributes(document);
  stripRemoteResourceControlAttributes(document);
  const allowedRemoteResourceCount =
    remoteContentMode === "allowed" ? preserveAllowedRemoteImages(document) : 0;
  const links = rewriteLinks(document);
  const blockedRemoteResourceCount = Math.max(
    0,
    remoteResourceReferenceCount - allowedRemoteResourceCount,
  );

  const html = DOMPurify.sanitize(
    document.body.innerHTML,
    sanitizerConfig,
  ) as string;

  return {
    html,
    plainText: document.body.textContent?.trim() ?? "",
    links,
    remoteContentBlocked:
      remoteResourceReferenceCount > 0 && blockedRemoteResourceCount > 0,
    blockedRemoteResourceCount,
    allowedRemoteResourceCount,
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

function stripRemoteResourceControlAttributes(document: Document) {
  for (const element of document.querySelectorAll("[loading], [referrerpolicy]")) {
    element.removeAttribute("loading");
    element.removeAttribute("referrerpolicy");
  }
}

export function isSafeOpenableUrl(value: string): URL | null {
  let parsed: URL;

  if (containsControlOrEncodedControl(value)) {
    return null;
  }

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

  if (parsed.protocol === "mailto:") {
    return isSafeMailtoUrl(parsed) ? parsed : null;
  }

  if (!parsed.hostname || LOCAL_OR_PRIVATE_HOST_PATTERN.test(parsed.hostname)) {
    return null;
  }

  return parsed;
}

export function isSafeRemoteImageUrl(value: string): URL | null {
  let parsed: URL;

  try {
    parsed = new URL(value);
  } catch {
    return null;
  }

  if (parsed.protocol !== "https:") {
    return null;
  }

  if (!parsed.hostname || parsed.username || parsed.password) {
    return null;
  }

  if (LOCAL_OR_PRIVATE_HOST_PATTERN.test(parsed.hostname)) {
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

function preserveAllowedRemoteImages(document: Document): number {
  let allowedCount = 0;

  for (const element of document.querySelectorAll("[src]")) {
    const src = element.getAttribute("src");
    const parsed =
      element.tagName.toLowerCase() === "img" && src
        ? isSafeRemoteImageUrl(src)
        : null;

    if (!parsed) {
      element.removeAttribute("src");
      continue;
    }

    element.setAttribute("src", parsed.href);
    element.setAttribute("loading", "lazy");
    element.setAttribute("referrerpolicy", "no-referrer");
    allowedCount += 1;
  }

  return allowedCount;
}

function isSafeMailtoUrl(parsed: URL) {
  if (parsed.host || parsed.pathname.startsWith("//")) {
    return false;
  }

  let hasRecipient = false;
  if (parsed.pathname.trim()) {
    if (!isSafeMailtoAddressList(parsed.pathname)) {
      return false;
    }
    hasRecipient = true;
  }

  for (const [key, value] of parsed.searchParams) {
    const normalizedKey = key.toLowerCase();
    if (
      !MAILTO_ALLOWED_FIELDS.has(normalizedKey) ||
      containsControlCharacter(key) ||
      containsControlCharacter(value)
    ) {
      return false;
    }

    if (["bcc", "cc", "to"].includes(normalizedKey)) {
      if (!isSafeMailtoAddressList(value)) {
        return false;
      }
      hasRecipient = true;
    }
  }

  return hasRecipient;
}

function isSafeMailtoAddressList(value: string) {
  if (value.includes("%")) {
    return false;
  }

  const addresses = value.split(",").map((address) => address.trim());
  return (
    addresses.length > 0 &&
    addresses.every((address) => /^[^@\s<>]+@[^@\s<>]+\.[^@\s<>]+$/.test(address))
  );
}

export function hasRemoteContentReferences(input: string | null | undefined) {
  return Boolean(input && countRemoteResourceReferences(input) > 0);
}

function countRemoteResourceReferences(input: string): number {
  return Array.from(input.matchAll(REMOTE_RESOURCE_PATTERN)).length;
}

function containsControlOrEncodedControl(value: string) {
  const lowered = value.toLowerCase();
  return (
    containsControlCharacter(value) ||
    lowered.includes("%00") ||
    lowered.includes("%0a") ||
    lowered.includes("%0d")
  );
}

function containsControlCharacter(value: string) {
  return Array.from(value).some((character) => {
    const codePoint = character.codePointAt(0);
    return codePoint !== undefined && (codePoint < 32 || codePoint === 127);
  });
}
