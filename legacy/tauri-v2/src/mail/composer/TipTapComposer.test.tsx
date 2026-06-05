import { Editor } from "@tiptap/core";
import { describe, expect, it } from "vitest";
import {
  composerEditorExtensions,
  sanitizeComposerHtml,
} from "./composerRichText";

function editorWithContent(content: string) {
  return new Editor({
    content,
    element: document.createElement("div"),
    extensions: composerEditorExtensions,
  });
}

describe("TipTap composer rich formatting", () => {
  it("serializes and sanitizes controlled font, color, highlight, and alignment styles", () => {
    const editor = editorWithContent("<p>Rich formatting</p>");

    editor.commands.selectAll();
    editor.commands.setTextAlign("center");
    editor.commands.setFontFamily("Georgia");
    editor.commands.setFontSize("18px");
    editor.commands.setHighlight({ color: "#fff2a8" });
    editor.commands.setColor("#9a624f");
    editor.commands.toggleStrike();

    const html = sanitizeComposerHtml(editor.getHTML());

    expect(html).toContain("Rich formatting");
    expect(html).toContain("<s>");
    expect(html).toContain("text-align: center");
    expect(html).toContain("font-family: Georgia");
    expect(html).toContain("font-size: 18px");
    expect(html).toContain("color: #9a624f");
    expect(html).toContain("background-color: #fff2a8");
  });

  it("serializes and sanitizes tables and code blocks", () => {
    const editor = editorWithContent(`
      <table>
        <tbody>
          <tr><th><p>Item</p></th><th><p>Status</p></th></tr>
          <tr><td><p>Sync</p></td><td><p>Ready</p></td></tr>
        </tbody>
      </table>
      <pre><code>const synced = true;</code></pre>
      <p>Use <code>send_email</code> after validation.</p>
    `);

    const html = sanitizeComposerHtml(editor.getHTML());

    expect(html).toContain("<table>");
    expect(html).toContain("<th");
    expect(html).toContain("<td");
    expect(html).toContain("Sync");
    expect(html).toContain("<pre><code>const synced = true;</code></pre>");
    expect(html).toContain("<code>send_email</code>");
  });

  it("strips unsafe rich formatting styles while preserving whitelisted formatting", () => {
    const html = sanitizeComposerHtml(`
      <p style="text-align: center; position: absolute; background-image: url(javascript:alert(1))">
        <span style="color: #5d7567; font-size: 999px; font-family: Comic Sans MS">Styled</span>
        <span style="font-size: 18px; font-family: Georgia">Safe</span>
      </p>
      <table><tbody><tr><td colspan="99" onclick="alert(1)">Cell</td></tr></tbody></table>
    `);

    expect(html).toContain("text-align: center");
    expect(html).toContain("color: #5d7567");
    expect(html).toContain("font-size: 18px");
    expect(html).toContain("font-family: Georgia");
    expect(html).not.toContain("position");
    expect(html).not.toContain("background-image");
    expect(html).not.toContain("javascript:");
    expect(html).not.toContain("999px");
    expect(html).not.toContain("Comic Sans");
    expect(html).not.toContain("onclick");
    expect(html).not.toContain("colspan=\"99\"");
  });
});
