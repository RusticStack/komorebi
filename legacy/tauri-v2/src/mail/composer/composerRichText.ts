import StarterKit from "@tiptap/starter-kit";
import Link from "@tiptap/extension-link";
import Image from "@tiptap/extension-image";
import { Table } from "@tiptap/extension-table";
import TableCell from "@tiptap/extension-table-cell";
import TableHeader from "@tiptap/extension-table-header";
import TableRow from "@tiptap/extension-table-row";
import TextAlign from "@tiptap/extension-text-align";
import Highlight from "@tiptap/extension-highlight";
import Color from "@tiptap/extension-color";
import FontFamily from "@tiptap/extension-font-family";
import { FontSize, TextStyle } from "@tiptap/extension-text-style";
import DOMPurify, { type Config } from "dompurify";

export type AlignmentValue = "left" | "center" | "right" | "justify";

export const FONT_FAMILIES = [
  { labelKey: "composer.format.fontFamilies.system", value: "system-ui" },
  { labelKey: "composer.format.fontFamilies.arial", value: "Arial" },
  { labelKey: "composer.format.fontFamilies.georgia", value: "Georgia" },
  { labelKey: "composer.format.fontFamilies.times", value: "Times New Roman" },
  { labelKey: "composer.format.fontFamilies.mono", value: "Courier New" },
];
export const FONT_SIZES = ["12px", "14px", "16px", "18px", "20px", "24px"];
export const TEXT_COLORS = [
  { labelKey: "composer.format.textColors.charcoal", value: "#2d2d2d" },
  { labelKey: "composer.format.textColors.sage", value: "#5d7567" },
  { labelKey: "composer.format.textColors.terracotta", value: "#9a624f" },
  { labelKey: "composer.format.textColors.blue", value: "#315f8c" },
];
export const HIGHLIGHT_COLORS = [
  { labelKey: "composer.format.highlightColors.honey", value: "#fff2a8" },
  { labelKey: "composer.format.highlightColors.sage", value: "#dfeadf" },
  { labelKey: "composer.format.highlightColors.terracotta", value: "#f4d6ca" },
];

export const allowedFontFamilies = new Set(
  FONT_FAMILIES.map((option) => option.value),
);
export const allowedFontSizes = new Set(FONT_SIZES);
export const allowedStyleColors = new Set([
  ...TEXT_COLORS.map((option) => option.value),
  ...HIGHLIGHT_COLORS.map((option) => option.value),
]);
export const allowedAlignments = new Set<AlignmentValue>([
  "left",
  "center",
  "right",
  "justify",
]);

const composerSanitizerConfig: Config = {
  SANITIZE_NAMED_PROPS: true,
  ALLOW_ARIA_ATTR: true,
  ALLOW_DATA_ATTR: false,
  ALLOWED_TAGS: [
    "a",
    "blockquote",
    "br",
    "code",
    "em",
    "img",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "s",
    "span",
    "strong",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
  ],
  ALLOWED_ATTR: [
    "alt",
    "colspan",
    "href",
    "rel",
    "rowspan",
    "src",
    "style",
    "title",
  ],
  FORBID_ATTR: [
    "background",
    "form",
    "formaction",
    "onerror",
    "onload",
    "onclick",
    "poster",
    "srcdoc",
    "srcset",
    "target",
    "xlink:href",
  ],
};

export const composerEditorExtensions = [
  StarterKit.configure({
    heading: false,
    horizontalRule: false,
    link: false,
  }),
  TextStyle,
  Color.configure({ types: ["textStyle"] }),
  FontFamily.configure({ types: ["textStyle"] }),
  FontSize.configure({ types: ["textStyle"] }),
  Highlight.configure({ multicolor: true }),
  TextAlign.configure({
    alignments: Array.from(allowedAlignments),
    types: ["paragraph", "tableCell", "tableHeader"],
  }),
  Table.configure({
    resizable: false,
  }),
  TableRow,
  TableHeader,
  TableCell,
  Link.configure({
    autolink: false,
    openOnClick: false,
    linkOnPaste: true,
    defaultProtocol: "https",
  }),
  Image.configure({
    allowBase64: true,
  }),
];

export function sanitizeComposerHtml(
  input: string,
  inlineImageSources: Map<string, string> = new Map(),
) {
  const withCidImages = Array.from(inlineImageSources.entries()).reduce(
    (html, [dataUrl, contentId]) => html.split(dataUrl).join(`cid:${contentId}`),
    input,
  );

  if (!DOMPurify.isSupported) {
    return "";
  }

  const sanitized = DOMPurify.sanitize(
    withCidImages,
    composerSanitizerConfig,
  ) as string;
  const document = new DOMParser().parseFromString(sanitized, "text/html");

  for (const anchor of document.querySelectorAll("a[href]")) {
    const parsed = safeComposerLink(anchor.getAttribute("href") ?? "");
    if (!parsed) {
      anchor.removeAttribute("href");
      anchor.removeAttribute("rel");
      continue;
    }
    anchor.setAttribute("href", parsed.href);
    anchor.setAttribute("rel", "noreferrer noopener nofollow");
  }

  for (const image of document.querySelectorAll("img[src]")) {
    const src = image.getAttribute("src") ?? "";
    if (!isSafeComposerImageSrc(src)) {
      image.remove();
    }
  }

  for (const styledElement of document.querySelectorAll("[style]")) {
    const safeStyle = safeComposerStyle(styledElement.getAttribute("style") ?? "");
    if (safeStyle) {
      styledElement.setAttribute("style", safeStyle);
    } else {
      styledElement.removeAttribute("style");
    }
  }

  for (const tableCell of document.querySelectorAll("td, th")) {
    normalizeTableSpanAttribute(tableCell, "colspan");
    normalizeTableSpanAttribute(tableCell, "rowspan");
  }

  return DOMPurify.sanitize(
    document.body.innerHTML,
    composerSanitizerConfig,
  ) as string;
}

export function safeComposerLink(value: string): URL | null {
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

function safeComposerStyle(value: string) {
  const declarations = value
    .split(";")
    .map((declaration) => declaration.trim())
    .filter(Boolean);
  const safeDeclarations: string[] = [];

  for (const declaration of declarations) {
    const [rawProperty, ...rawValueParts] = declaration.split(":");
    const property = rawProperty?.trim().toLocaleLowerCase();
    const rawValue = rawValueParts.join(":").trim();
    const normalizedValue = normalizeStyleValue(rawValue);

    if (
      property === "text-align" &&
      allowedAlignments.has(normalizedValue as AlignmentValue)
    ) {
      safeDeclarations.push(`text-align: ${normalizedValue}`);
    } else if (
      property === "color" &&
      allowedStyleColors.has(normalizedValue)
    ) {
      safeDeclarations.push(`color: ${normalizedValue}`);
    } else if (
      property === "background-color" &&
      allowedStyleColors.has(normalizedValue)
    ) {
      safeDeclarations.push(`background-color: ${normalizedValue}`);
    } else if (
      property === "font-size" &&
      allowedFontSizes.has(normalizedValue)
    ) {
      safeDeclarations.push(`font-size: ${normalizedValue}`);
    } else if (
      property === "font-family" &&
      allowedFontFamilies.has(normalizeFontFamilyValue(rawValue))
    ) {
      safeDeclarations.push(`font-family: ${normalizeFontFamilyValue(rawValue)}`);
    }
  }

  return safeDeclarations.join("; ");
}

function normalizeStyleValue(value: string) {
  const trimmed = value.trim().toLocaleLowerCase();
  const rgbMatch = trimmed.match(
    /^rgb\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*\)$/,
  );
  if (!rgbMatch) {
    return trimmed;
  }

  const [, red, green, blue] = rgbMatch;
  return `#${[red, green, blue]
    .map((component) =>
      Math.min(255, Math.max(0, Number.parseInt(component, 10)))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function normalizeFontFamilyValue(value: string) {
  return value
    .split(",")[0]
    .trim()
    .replace(/^["']|["']$/g, "");
}

function normalizeTableSpanAttribute(element: Element, attribute: string) {
  const value = element.getAttribute(attribute);
  if (!value) {
    return;
  }

  const parsed = Number.parseInt(value, 10);
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 12) {
    element.removeAttribute(attribute);
  } else {
    element.setAttribute(attribute, String(parsed));
  }
}

function isSafeComposerImageSrc(value: string) {
  return (
    /^cid:[A-Za-z0-9_.@-]+$/.test(value) ||
    /^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/=]+$/.test(value)
  );
}
