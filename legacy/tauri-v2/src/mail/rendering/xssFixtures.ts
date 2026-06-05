export type XssFixture = {
  name: string;
  html: string;
};

export type SanitizerBypassFixture = XssFixture & {
  category:
    | "script"
    | "event"
    | "javascript-url"
    | "data-url"
    | "css"
    | "svg-mathml"
    | "malformed"
    | "post-sanitization-mutation";
  remoteResourceCount?: number;
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

export const sanitizerBypassFixtures: SanitizerBypassFixture[] = [
  {
    category: "script",
    name: "mixed-case script tag",
    html: "<ScRiPt>alert('xss')</ScRiPt><p>Message</p>",
  },
  {
    category: "script",
    name: "broken nested script token",
    html: "<scr<script>ipt>alert(1)</scr</script>ipt><p>After</p>",
  },
  {
    category: "event",
    name: "mixed-case event handler",
    html: '<p OnPointerEnter="alert(1)" onclick="alert(2)">Hover</p>',
  },
  {
    category: "event",
    name: "image load event handler",
    html: '<img src="https://tracker.example/open.png" oNLoAd="alert(1)" alt="open">',
    remoteResourceCount: 1,
  },
  {
    category: "javascript-url",
    name: "encoded javascript link",
    html: '<a href="jav&#x0D;ascript:alert(1)">Open</a>',
  },
  {
    category: "javascript-url",
    name: "mixed-case javascript image source",
    html: '<img src="JaVaScRiPt:alert(1)" alt="bad">',
  },
  {
    category: "data-url",
    name: "data html link",
    html: '<a href="data:text/html,<script>alert(1)</script>">Report</a>',
  },
  {
    category: "data-url",
    name: "data svg image source",
    html: '<img src="data:image/svg+xml,<svg onload=alert(1)>" alt="inline">',
    remoteResourceCount: 1,
  },
  {
    category: "css",
    name: "style tag import and javascript url",
    html: "<style>@import url(https://tracker.example/mail.css); p{background:url(javascript:alert(1))}</style><p>Styled</p>",
    remoteResourceCount: 1,
  },
  {
    category: "css",
    name: "inline css expression and remote image",
    html: '<p style="width: expression(alert(1)); background-image: url(https://tracker.example/bg.png)">Styled</p>',
    remoteResourceCount: 1,
  },
  {
    category: "svg-mathml",
    name: "svg foreignObject event handler",
    html: '<svg><foreignObject><img src="x" onerror="alert(1)"></foreignObject></svg><p>Fallback</p>',
  },
  {
    category: "svg-mathml",
    name: "math annotation script payload",
    html: '<math><annotation-xml encoding="text/html"><script>alert(1)</script><p>Math text</p></annotation-xml></math><p>After math</p>',
  },
  {
    category: "svg-mathml",
    name: "svg xlink javascript href",
    html: '<svg><a xlink:href="javascript:alert(1)"><text>Click</text></a></svg><p>Safe text</p>',
  },
  {
    category: "malformed",
    name: "unclosed tags with event handler",
    html: '<div><p title="unterminated"><img src="https://tracker.example/pixel.png" onerror="alert(1)"><span>Body',
    remoteResourceCount: 1,
  },
  {
    category: "malformed",
    name: "null character protocol split",
    html: '<a href="java\u0000script:alert(1)">Null split</a>',
  },
  {
    category: "post-sanitization-mutation",
    name: "math svg style mutation payload",
    html: '<math><mtext></form><form><mglyph><svg><mtext><style><a title="</style><img src=x onerror=alert(1)>">Mutation</a></style></mtext></svg></mglyph></form></mtext></math><p>After</p>',
  },
  {
    category: "post-sanitization-mutation",
    name: "reserved komorebi link metadata injection",
    html: '<a data-komorebi-url="https://evil.example" data-komorebi-link-id="email-link-1">Trap</a><a href="https://example.com/safe">Safe</a>',
  },
];
