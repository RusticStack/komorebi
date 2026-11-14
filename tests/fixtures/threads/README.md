# Long reply chains

Synthetic business threads (no real mail): each message quotes the whole
earlier thread in the style of the client that sent the reply (Apple Mail and
Thunderbird `blockquote type="cite"`, Gmail `gmail_quote`, Outlook desktop
`MsoNormal` with the From/Sent/To/Subject block, OWA `divRplyFwdMsg`, and
forwards inside replies), with Word styles, `o:p`, conditional comments,
signatures with a logo, long disclaimers, `font-size:1pt` spacers and
`line-height` in px, em, unitless and percent.

| File | Levels | HTML |
|---|---|---|
| t20-thread.html | 20 | 63 KB |
| t40-thread.html | 40 | 133 KB |
| t60-thread.html | 60 | 198 KB |
| t60-big-thread.html | 60, three times the text per reply | 546 KB |
| t60-wide-thread.html | t60 with a 900 px wide forwarded table a third of the way in | 204 KB |

`python gen_threads.py <dir>` writes the first four (and `.eml` copies with a
text part and the logo as a `cid:` image); the wide one adds the table to
t60-thread. `tests/render_threads.rs` lays them out.
