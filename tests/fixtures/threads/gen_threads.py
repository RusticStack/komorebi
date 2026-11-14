"""Synthetic long business reply chains (nothing real). gen_threads.py <outdir>

Each message quotes the whole earlier thread, in the quoting style of the client
that sent that reply: Apple Mail / Thunderbird nested blockquote type=cite, Gmail
gmail_quote, Outlook desktop (divRplyFwdMsg / hr + From/Sent/To/Subject, MsoNormal,
mso-* styles, <o:p>, conditional comments), OWA, and forwards inside replies.
Writes <name>.html (the body) and <name>.eml (multipart/alternative with a text
part and a multipart/related HTML part with a logo).
"""
import base64, io, pathlib, random, sys, zlib, struct

out = pathlib.Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)

PEOPLE = [
    ("Ana Ribeiro", "ana.ribeiro@supplier.example", "Supplier Lda."),
    ("Mark Jensen", "mark.jensen@buyer.example", "Buyer Inc."),
    ("Joana Costa", "joana.costa@supplier.example", "Supplier Lda."),
    ("Peter Novak", "p.novak@logistics.example", "Logistics GmbH"),
    ("Sofia Almeida", "sofia@law.example", "Almeida & Partners"),
]
WORDS = ("shipment invoice pallet delivery schedule quote revision contract clause "
         "warehouse customs approval forecast volume pricing discount margin batch "
         "inspection sample tooling drawing tolerance lead time week order").split()
CLIENTS = ["apple", "gmail", "outlook", "owa", "thunderbird", "outlook", "gmail", "apple", "fwd"]

DISCLAIMER = (
    "CONFIDENTIALITY NOTICE: This e-mail message, including any attachments, is for the sole use "
    "of the intended recipient(s) and may contain confidential and privileged information. Any "
    "unauthorized review, use, disclosure or distribution is prohibited. If you are not the intended "
    "recipient, please contact the sender by reply e-mail and destroy all copies of the original "
    "message. Este e-mail e quaisquer anexos são confidenciais e destinados exclusivamente ao "
    "destinatário. Se recebeu esta mensagem por engano, por favor informe o remetente e apague-a. "
    "Please consider the environment before printing this e-mail. ") * 2


def para(r, n):
    w = [r.choice(WORDS) for _ in range(n)]
    w[0] = w[0].capitalize()
    return " ".join(w) + "."


def png_logo():
    w, h = 120, 40
    row = b"\x00" + b"".join(bytes((20 + x, 90, 160 - x // 2)) for x in range(w))
    raw = row * h
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) +
            chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def date(i):
    return f"Mon, {1 + i % 28:02d} Sep 2026 {8 + i % 10:02d}:{(i * 7) % 60:02d}"


def body_html(r, i, who, style):
    """The new text of message i, in a client's markup."""
    name, mail, org = who
    paras = [para(r, r.randrange(12, 60)) for _ in range(r.randrange(2, 6))]
    if style in ("outlook", "owa"):
        fs = r.choice(["11.0pt", "10.0pt", "12.0pt"])
        p = "".join(f'<p class="MsoNormal"><span style="font-size:{fs};font-family:&quot;Calibri&quot;,sans-serif;'
                    f'mso-fareast-language:EN-US">{t}<o:p></o:p></span></p>'
                    f'<p class="MsoNormal"><span style="font-size:{fs}"><o:p>&nbsp;</o:p></span></p>' for t in paras)
        table = ""
        if r.random() < 0.4:
            rows = "".join(f'<tr><td style="border:solid #BFBFBF 1.0pt;padding:0cm 5.4pt 0cm 5.4pt"><p class="MsoNormal">'
                           f'{r.choice(WORDS)}<o:p></o:p></p></td><td style="border:solid #BFBFBF 1.0pt;border-left:none;'
                           f'padding:0cm 5.4pt"><p class="MsoNormal" align="right" style="text-align:right">{r.randrange(10, 9999)}'
                           f'<o:p></o:p></p></td></tr>' for _ in range(r.randrange(3, 8)))
            table = (f'<table class="MsoTableGrid" border="1" cellspacing="0" cellpadding="0" style="border-collapse:collapse;'
                     f'border:none;mso-border-alt:solid windowtext .5pt">{rows}</table>')
        sig = (f'<p class="MsoNormal"><b><span style="font-size:10.0pt;color:#1F3864">{name}</span></b>'
               f'<span style="font-size:10.0pt;color:#1F3864"><br>{org}<br>+351 210 000 {i:03d}</span><o:p></o:p></p>'
               f'<p class="MsoNormal"><img width="120" height="40" style="width:1.25in;height:.4166in" src="cid:logo" alt="{org}"><o:p></o:p></p>'
               f'<p class="MsoNormal" style="line-height:115%"><span style="font-size:7.0pt;line-height:115%;color:gray">{DISCLAIMER}</span><o:p></o:p></p>')
        return f'<div class="WordSection1">{p}{table}{sig}</div>'
    if style == "gmail":
        p = "".join(f"<div>{t}</div><div><br></div>" for t in paras)
        sig = (f'<div><br clear="all"><div><div dir="ltr" class="gmail_signature"><div style="font-size:12.8px">'
               f'<b>{name}</b></div><div style="font-size:small;line-height:1.5">{org}</div>'
               f'<img src="cid:logo" width="96" height="32"><div style="font-size:1pt;line-height:1pt">&nbsp;</div>'
               f'<div style="font-size:9px;line-height:12px;color:#888">{DISCLAIMER}</div></div></div></div>')
        return f'<div dir="ltr">{p}{sig}</div>'
    # apple, thunderbird, fwd
    lh = r.choice(["1.4", "18px", "1.2em", "130%", "normal"])
    fs = r.choice(["13px", "14px", "12pt", "small"])
    p = "".join(f'<p style="margin:0;font-size:{fs};line-height:{lh}">{t}</p><p style="margin:0"><br></p>' for t in paras)
    sig = (f'<div style="font-family:Helvetica;font-size:12px">-- <br>{name}<br>{org}</div>'
           f'<div style="font-size:0;line-height:0"><img src="cid:logo" width="60" height="20"></div>'
           f'<div style="font-size:8pt;line-height:10pt;color:#777">{DISCLAIMER[:600]}</div>')
    return f'<div style="font-family:-apple-system,Helvetica,Arial">{p}{sig}</div>'


def quote(style, i, who, inner):
    """Message i's reply header and the quoted earlier thread, in a client's markup."""
    name, mail, _ = who
    if style in ("apple", "thunderbird"):
        att = (f'<div>On {date(i)}, {name} &lt;<a href="mailto:{mail}">{mail}</a>&gt; wrote:</div><br>'
               if style == "apple" else
               f'<div class="moz-cite-prefix">On {date(i)} {name} wrote:<br></div>')
        return f'{att}<blockquote type="cite" style="margin:0 0 0 .8ex;border-left:1px solid #ccc;padding-left:1ex">{inner}</blockquote>'
    if style == "gmail":
        return (f'<br><div class="gmail_quote"><div dir="ltr" class="gmail_attr">On {date(i)} {name} &lt;'
                f'<a href="mailto:{mail}">{mail}</a>&gt; wrote:<br></div>'
                f'<blockquote class="gmail_quote" style="margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);'
                f'padding-left:1ex">{inner}</blockquote></div>')
    if style == "outlook":
        return (f'<p class="MsoNormal"><o:p>&nbsp;</o:p></p><div><div style="border:none;border-top:solid #E1E1E1 1.0pt;'
                f'padding:3.0pt 0cm 0cm 0cm"><p class="MsoNormal"><b><span lang="EN-US">From:</span></b>'
                f'<span lang="EN-US"> {name} &lt;{mail}&gt; <br><b>Sent:</b> {date(i)}<br><b>To:</b> '
                f'{PEOPLE[1][0]} &lt;{PEOPLE[1][1]}&gt;<br><b>Subject:</b> RE: Order 4471 delivery schedule<o:p></o:p>'
                f'</span></p></div></div><p class="MsoNormal"><o:p>&nbsp;</o:p></p>{inner}')
    if style == "owa":
        return (f'<div id="appendonsend"></div><hr style="display:inline-block;width:98%" tabindex="-1">'
                f'<div id="divRplyFwdMsg" dir="ltr"><font face="Calibri, sans-serif" style="font-size:11pt" color="#000000">'
                f'<b>From:</b> {name} &lt;{mail}&gt;<br><b>Sent:</b> {date(i)}<br><b>To:</b> {PEOPLE[1][0]}<br>'
                f'<b>Subject:</b> RE: Order 4471 delivery schedule</font><div>&nbsp;</div></div>{inner}')
    # A forward inside the reply: the forwarded header block, then the thread unquoted.
    return (f'<div><br></div><div>Begin forwarded message:</div><br><blockquote type="cite"><div style="margin:0">'
            f'<span style="font-family:-webkit-system-font,Helvetica;font-size:13px"><b>From: </b>{name} &lt;{mail}&gt;'
            f'<br><b>Subject: </b>Fwd: Order 4471<br><b>Date: </b>{date(i)}<br></span></div><br>{inner}</blockquote>')


def thread(levels, seed, scale=1):
    r = random.Random(seed)
    html = ""
    for i in range(levels):
        style = CLIENTS[(i + seed) % len(CLIENTS)]
        who = PEOPLE[i % len(PEOPLE)]
        new = "".join(body_html(r, i, who, style) for _ in range(scale))
        html = new + (quote(style, i, PEOPLE[(i + 1) % len(PEOPLE)], html) if html else "")
    head = ('<html xmlns:o="urn:schemas-microsoft-com:office:office"><head><meta http-equiv="Content-Type" content="text/html; charset=utf-8">'
            '<!--[if gte mso 9]><xml><o:OfficeDocumentSettings><o:AllowPNG/></o:OfficeDocumentSettings></xml><![endif]-->'
            '<style><!-- p.MsoNormal, li.MsoNormal, div.MsoNormal {margin:0cm;font-size:11.0pt;font-family:"Calibri",sans-serif;} '
            'a:link {color:#0563C1;text-decoration:underline;} span.EmailStyle17 {mso-style-type:personal-compose;} '
            '.MsoChpDefault {mso-style-type:export-only;} @page WordSection1 {size:612.0pt 792.0pt;} '
            'div.WordSection1 {page:WordSection1;} --></style></head>'
            '<body lang="EN-US" link="#0563C1" vlink="#954F72" style="word-wrap:break-word">')
    return head + html + "</body></html>"


def text_of(html):
    import re
    t = re.sub(r"<(br|/p|/div|tr)[^>]*>", "\n", html)
    t = re.sub(r"<[^>]+>", "", t)
    return re.sub(r"\n{3,}", "\n\n", t.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", '"'))


def eml(name, subject, html, logo):
    b1, b2 = "=_alt_boundary", "=_rel_boundary"
    def b64(d):
        s = base64.b64encode(d).decode()
        return [s[i:i + 76] for i in range(0, len(s), 76)]
    lines = ["From: Ana Ribeiro <ana.ribeiro@supplier.example>", "To: Mark Jensen <mark.jensen@buyer.example>",
             f"Subject: {subject}", "Date: Mon, 02 Nov 2026 10:00:00 +0000", "MIME-Version: 1.0",
             f'Content-Type: multipart/alternative; boundary="{b1}"', "",
             f"--{b1}", "Content-Type: text/plain; charset=utf-8", "Content-Transfer-Encoding: base64", "",
             *b64(text_of(html).encode()),
             f"--{b1}", f'Content-Type: multipart/related; boundary="{b2}"', "",
             f"--{b2}", "Content-Type: text/html; charset=utf-8", "Content-Transfer-Encoding: base64", "",
             *b64(html.encode()),
             f"--{b2}", "Content-Type: image/png", "Content-Transfer-Encoding: base64", "Content-ID: <logo>",
             'Content-Disposition: inline; filename="logo.png"', "", *b64(logo),
             f"--{b2}--", f"--{b1}--"]
    (out / f"{name}.eml").write_text("\r\n".join(lines) + "\r\n", encoding="ascii", newline="")


logo = png_logo()
for name, levels, seed, scale in [("t20-thread", 20, 1, 1), ("t40-thread", 40, 2, 1),
                                  ("t60-thread", 60, 3, 1), ("t60-big-thread", 60, 4, 3)]:
    h = thread(levels, seed, scale)
    (out / f"{name}.html").write_text(h, encoding="utf-8")
    eml(name, f"RE: Order 4471 delivery schedule ({levels} replies)", h, logo)
    print(name, len(h.encode()))
