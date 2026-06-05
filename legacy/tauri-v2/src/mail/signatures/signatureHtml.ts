import DOMPurify, { type Config } from "dompurify";
import { isSafeOpenableUrl } from "../rendering/emailHtml";

export type SanitizedSignatureHtml = {
  html: string;
  plainText: string;
};

export const SIGNATURE_HTML_BLOCK_START = "<!-- komorebi-signature:start -->";
export const SIGNATURE_HTML_BLOCK_END = "<!-- komorebi-signature:end -->";

const SIGNATURE_SANITIZER_CONFIG: Config = {
  SANITIZE_NAMED_PROPS: true,
  ALLOW_ARIA_ATTR: true,
  ALLOW_DATA_ATTR: false,
  ALLOWED_TAGS: [
    "a",
    "b",
    "blockquote",
    "br",
    "div",
    "em",
    "i",
    "li",
    "ol",
    "p",
    "span",
    "strong",
    "u",
    "ul",
  ],
  ALLOWED_ATTR: ["aria-label", "href", "rel", "title"],
  FORBID_ATTR: [
    "background",
    "form",
    "formaction",
    "poster",
    "src",
    "srcdoc",
    "srcset",
    "style",
    "target",
    "xlink:href",
  ],
};

export function sanitizeSignatureHtml(input: string): SanitizedSignatureHtml {
  if (!DOMPurify.isSupported) {
    return {
      html: "",
      plainText: input,
    };
  }

  const sanitized = DOMPurify.sanitize(
    input.trim(),
    SIGNATURE_SANITIZER_CONFIG,
  ) as string;
  const document = new DOMParser().parseFromString(sanitized, "text/html");

  for (const anchor of document.querySelectorAll("a[href]")) {
    const href = anchor.getAttribute("href");
    const parsed = href ? isSafeOpenableUrl(href) : null;

    if (!parsed) {
      anchor.removeAttribute("href");
      anchor.removeAttribute("rel");
      continue;
    }

    anchor.setAttribute("href", parsed.href);
    anchor.setAttribute("rel", "noreferrer noopener nofollow");
    anchor.removeAttribute("target");
  }

  const html = DOMPurify.sanitize(
    document.body.innerHTML,
    SIGNATURE_SANITIZER_CONFIG,
  ) as string;

  return {
    html,
    plainText: document.body.textContent?.trim() ?? "",
  };
}

export function insertSignatureIntoComposerHtml(
  bodyHtml: string,
  signatureHtml: string | null | undefined,
) {
  const bodyWithoutSignature = removeExistingSignatureBlock(bodyHtml).trimEnd();
  const sanitizedSignature = signatureHtml
    ? sanitizeSignatureHtml(signatureHtml).html.trim()
    : "";

  if (!sanitizedSignature) {
    return bodyWithoutSignature;
  }

  const block = `${SIGNATURE_HTML_BLOCK_START}<div class="komorebi-account-signature">${sanitizedSignature}</div>${SIGNATURE_HTML_BLOCK_END}`;

  return bodyWithoutSignature ? `${bodyWithoutSignature}\n${block}` : block;
}

export function signatureHtmlToPlainText(signatureHtml: string) {
  return sanitizeSignatureHtml(signatureHtml).plainText;
}

function removeExistingSignatureBlock(bodyHtml: string) {
  let output = bodyHtml;
  let start = output.indexOf(SIGNATURE_HTML_BLOCK_START);

  while (start >= 0) {
    const afterStart = start + SIGNATURE_HTML_BLOCK_START.length;
    const relativeEnd = output
      .slice(afterStart)
      .indexOf(SIGNATURE_HTML_BLOCK_END);

    if (relativeEnd < 0) {
      break;
    }

    const end =
      afterStart + relativeEnd + SIGNATURE_HTML_BLOCK_END.length;
    output = `${output.slice(0, start)}${output.slice(end)}`;
    start = output.indexOf(SIGNATURE_HTML_BLOCK_START);
  }

  return output;
}
