# Synthetic HTML mail corpus

Fixtures for the native HTML renderer. **Everything here is synthetic. None of it is real mail.**

- 14 `NN-name.eml` files: full RFC 5322 / MIME messages with a `text/plain` alternative, the `text/html` body and the inline images as `multipart/related` parts (`Content-ID`, `Content-Disposition: inline`). The HTML and the images are inside the `.eml` files only.
- `_manifest.json`: machine-readable list of each message's description and its `cid:` to image map.
- `index.md`: the generator's description of each message (charset, transfer encoding, MIME tree, what it stresses). It also mentions the decoded `.html` copies and the `images/` folder; those were not copied because the `.eml` files already contain them.

Only `example.com`, `example.org`, `example.net` and `*.invalid` domains are used, and every remote URL points at `*.example.invalid`. Tests must never fetch them.

The corpus was produced by a throwaway Python generator (`corpus-gen/gen.py`, with `make_images.ps1` for the images and `verify.py` for the checks). It is not part of this repository. Treat these files as fixed inputs; do not regenerate or hand-edit them.

The `.eml` files are marked `-text` in `.gitattributes` so their CRLF line endings and 8-bit bytes (`14-adversarial-malformed.eml`) survive checkout unchanged.
