# Provider icons

The icons the account setup's provider list shows beside each built-in provider
(`src/providers.rs`). Each is the provider's own favicon or app icon, downloaded once by hand
on 2026-11-14 from the provider's official site, then fitted into a square and saved as a
palette PNG by `scripts/provider-icons.js`: `<id>.png` at 32 x 32 and `<id>@2x.png` at 64 x 64.
They are compiled into `komorebi.exe` (`crates/komorebi-gpui/src/views/onboarding/providers.rs`);
the app never fetches an icon. A provider without an icon gets a neutral monogram.

**Trademarks.** These names and icons are the trademarks of their owners. Komorebi uses them
only to identify each service in the list of mail providers it can connect to (nominative
use). Komorebi is not affiliated with, sponsored or endorsed by any of them.

| File | Provider | Downloaded from | Found on |
|---|---|---|---|
| `aol` | AOL Mail | https://mail.aol.com/f/apple-touch-icon.png | https://mail.aol.com/ |
| `fastmail` | Fastmail | https://www.fastmail.com/favicon.svg | https://www.fastmail.com/ |
| `free` | Free | https://www.free.fr/favicon.ico (a 60 px PNG) | https://www.free.fr/ |
| `gmail` | Gmail | https://www.gstatic.com/images/branding/productlogos/gmail_2026/v2/web/192px.svg | https://www.google.com/gmail/about/ |
| `gmx` | GMX (gmx.net and gmx.com) | https://s.uicdn.com/mampkg/@mamdev/core.assets/gmx/favicon.svg | https://www.gmx.net/ |
| `google-workspace` | Google Workspace | https://www.gstatic.com/images/branding/googleg/1x/googleg_standard_color_128dp.png | Google's own "G" (workspace.google.com links only a 20 px one) |
| `icloud` | iCloud Mail | https://www.icloud.com/system/icloud.com/2642Build23/favicons/default-favicon-light-180x180.png | https://www.icloud.com/ |
| `infomaniak` | Infomaniak | https://www.infomaniak.com/favicon.ico (its 64 px image) | https://www.infomaniak.com/ |
| `libero` | Libero Mail | https://www.libero.it/public/e/l/img/fi/icon.svg | https://www.libero.it/ |
| `mailboxorg` | mailbox.org | https://mailbox.org/themes/box/assets/favicon-width-background.svg | https://mailbox.org/ |
| `mailcom` | mail.com | https://www.mail.com/apple-touch-icon.png | https://www.mail.com/ |
| `orange` | Orange | https://hporange.cdn.s.woopic.com/sg1/apple-touch-icon.png | https://www.orange.fr/ |
| `outlook` | Outlook.com and Microsoft 365 | https://res.public.onecdn.static.microsoft/assets/mail/pwa/v4/pngs/Outlook.512x512.png | the web manifest of https://outlook.live.com/mail/ |
| `postale` | Postale.io | https://postale.io/images/favicon.svg | https://postale.io/ |
| `posteo` | Posteo | https://posteo.de/images/app-icons/apple-touch-icon.png | https://posteo.de/ |
| `proton` | Proton Mail | https://pmecdn.protonweb.com/image-transformation/?s=c&image=image/upload/v1776940263/static/logos/mail/mail-badge.svg | https://proton.me/mail |
| `purelymail` | Purelymail | https://purelymail.com/favicon.ico | https://purelymail.com/ |
| `seznam` | Seznam Email | https://login.seznam.cz/static/img/apple-touch-icon.png | https://email.seznam.cz/ |
| `webde` | WEB.DE | https://img.ui-portal.de/webde/favicon.ico (its 48 px image) | https://web.de/ |
| `yahoo` | Yahoo Mail | https://www.yahoo.com/apple-touch-icon.png | https://www.yahoo.com/ (mail.yahoo.com links only a 32 px icon) |
| `yandex` | Yandex Mail | https://yastatic.net/s3/psf/ps-tuning/a128d6c206b66fcf3154.png | https://360.yandex.com/mail/ |
| `zoho` | Zoho Mail | https://www.zohowebstatic.com/sites/zweb/images/productlogos/mail.svg (cropped to its envelope, `ZOHO_CROP`) | https://www.zoho.com/mail/ |

The Zoho Mail EU preset uses `zoho`. To refresh an icon, download it again into a new folder,
run the script and check the preview it can write (light and dark grounds) before committing.
