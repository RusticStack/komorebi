# Message Rendering Boundary

Last updated: 2026-05-22

## Current State

Komorebi renders untrusted email HTML only through `src/mail/rendering/SafeEmailBody.tsx`. The sanitizer and link/resource policy live in `src/mail/rendering/emailHtml.ts`.

The current app shell has no selected message and therefore renders no message HTML. When a message-detail view is connected, it must use this boundary and must not add any other `dangerouslySetInnerHTML` sink.

## Sanitization Policy

The frontend uses DOMPurify for email HTML sanitization. The project config:

- Uses the HTML profile only.
- Enables DOM named-property sanitization.
- Removes scripts, style tags, forms, inputs, iframes, objects, embeds, SVG, MathML, base tags, and metadata/link tags.
- Removes style attributes, event handler attributes, resource-loading attributes, `srcdoc`, `formaction`, and `xlink:href`.
- Runs a second sanitizer pass after Komorebi link rewriting.

The sanitizer output may be inserted into React only by `SafeEmailBody`.

## Remote Content

Remote and inline-resource loading is blocked by default. The renderer removes `src`, `srcset`, `poster`, and `background`, including `http`, `https`, protocol-relative, `cid`, and `data` resource references.

Future inline CID image support must map a parsed MIME attachment to an app-generated local preview URL through a typed Rust command and `docs/architecture/attachment-safety.md`. Sender-provided URLs and filenames must never become resource URLs directly.

Remote-content preferences, if added, must be scoped narrowly to the sender or message and must not weaken sanitizer rules.

## Links

Links are not left as direct navigations. The renderer:

- Allows only `http`, `https`, and `mailto` URLs.
- Rejects embedded credentials.
- Rewrites safe links to `href="#"` with `data-komorebi-url`.
- Adds `rel="noreferrer noopener nofollow"`.
- Blocks `javascript:`, `data:`, `file:`, and malformed URLs.
- Delegates clicks through `SafeEmailBody` so a future opener command can validate and ask for user intent.

The Tauri opener plugin remains denied. Adding external open behavior requires a typed backend command or narrowly scoped capability and tests.

## Plain Text Fallback

`SafeEmailBody` renders plain text when `body_html` is missing or sanitizes to empty output. Plain text is rendered through React text output, not `innerHTML`, so markup-like text is displayed as text.

## Fixtures And Tests

`src/mail/rendering/xssFixtures.ts` includes hostile HTML fixtures for script tags, event handlers, JavaScript/data links, SVG, iframe `srcdoc`, remote background loading, inline style execution, and forms.

`src/mail/rendering/SafeEmailBody.test.tsx` verifies:

- Hostile fixture tags and attributes are removed.
- Remote resource attributes are removed and reported as blocked.
- Safe links are rewritten for delegated opening.
- Unsafe links are stripped.
- Plain text fallback does not interpret markup.

`pnpm check` runs these tests through Vitest.

## References

- OWASP XSS Prevention Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html
- DOMPurify: https://github.com/cure53/DOMPurify
