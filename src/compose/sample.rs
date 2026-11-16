//! Made-up formatted content with pictures for the app's snapshot actions
//! (`KOMOREBI_SNAPSHOT_ACTION=compose-pictures`, `signature-pictures`;
//! only under `KOMOREBI_OFFLINE`): a two-step how-to and a signature
//! pasted from Outlook, its pictures generated here as plain shapes. No
//! real person, organisation, logo or mail.

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

use crate::mail::rich::edit::{Editor, OBJ};
use crate::mail::rich::{self as model, ImageFile, Images, InlineImage};

/// A placeholder picture: a light card with the given bars and blocks
/// (x, y, w, h, colour) on it, as a PNG.
fn picture(w: u32, h: u32, ground: [u8; 3], shapes: &[(f32, f32, f32, f32, [u8; 3])]) -> Vec<u8> {
    let Some(mut pm) = Pixmap::new(w, h) else {
        return Vec::new();
    };
    pm.fill(Color::from_rgba8(ground[0], ground[1], ground[2], 255));
    for &(x, y, sw, sh, c) in shapes {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], 255);
        paint.anti_alias = true;
        if let Some(r) = Rect::from_xywh(x, y, sw, sh) {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
    }
    pm.encode_png().unwrap_or_default()
}

/// A round mark (a placeholder seal), as a PNG.
fn seal(size: u32, ring: [u8; 3], fill: [u8; 3]) -> Vec<u8> {
    let Some(mut pm) = Pixmap::new(size, size) else {
        return Vec::new();
    };
    pm.fill(Color::WHITE);
    let s = size as f32;
    for (radius, c) in [(s * 0.46, ring), (s * 0.34, fill)] {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], 255);
        paint.anti_alias = true;
        if let Some(path) = PathBuilder::from_circle(s / 2., s / 2., radius) {
            pm.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    pm.encode_png().unwrap_or_default()
}

/// A dialog mock-up (title bar, two fields, a button) and a list
/// mock-up (rows, one picked).
fn how_to_pictures() -> [Vec<u8>; 2] {
    let grey = [228, 230, 234];
    let ink = [60, 64, 72];
    let accent = [70, 110, 170];
    let dialog = picture(
        520,
        300,
        [248, 248, 250],
        &[
            (0., 0., 520., 40., grey),
            (24., 14., 160., 12., ink),
            (24., 80., 472., 36., grey),
            (24., 140., 472., 36., grey),
            (376., 236., 120., 40., accent),
        ],
    );
    let list = picture(
        420,
        260,
        [248, 248, 250],
        &[
            (20., 20., 380., 40., grey),
            (20., 70., 380., 40., [210, 222, 240]),
            (20., 120., 380., 40., grey),
            (20., 170., 380., 40., grey),
            (36., 84., 200., 12., ink),
        ],
    );
    [dialog, list]
}

/// A two-step how-to in `ed` (its text replaced): a line, a picture, a
/// line, a picture. The offset of the first picture.
pub fn how_to(ed: &mut Editor) -> Option<usize> {
    let one = "Step 1: open the Print dialog.";
    let two = "Step 2: pick the printer, then select Print.";
    let len = ed.text().len();
    let text = format!("{one}\n\n{two}\n\n");
    ed.replace(0, len, &text, (0, len))?;
    let [a, b] = how_to_pictures();
    let mut at = one.len() + 1;
    let mut first = None;
    for (bytes, alt) in [(a, "The Print dialog"), (b, "The printer list")] {
        let (file, w, h) = ImageFile::new(bytes)?;
        let img = InlineImage::new(file.cid(), w, h, alt).sized(u16::try_from(w / 2).ok()?);
        ed.insert_image(at, at, img, file)?;
        first.get_or_insert(at);
        at += OBJ.len_utf8() + 1 + two.len() + 1;
    }
    first
}

/// A signature as Outlook puts it on the clipboard: a layout table (a
/// logo on the left, the name and lines on the right, valign top), a row
/// of partner logos under it, a link, and one picture on the web (left
/// out: nothing is fetched). Its name, the document and its pictures.
pub fn signature() -> (String, model::Doc, Images) {
    let uri = |bytes: Vec<u8>| -> String {
        ImageFile::new(bytes).map_or_else(String::new, |(f, _, _)| {
            crate::mail::rich::images::data_uri(&f)
        })
    };
    let logo = uri(seal(96, [70, 110, 170], [230, 236, 246]));
    let partner = |c: [u8; 3]| uri(picture(120, 40, [255, 255, 255], &[(8., 8., 104., 24., c)]));
    let (p1, p2, p3) = (
        partner([120, 160, 110]),
        partner([190, 140, 90]),
        partner([140, 120, 180]),
    );
    let html = format!(
        r#"<html><body><table border="0" cellspacing="0" cellpadding="0" style="border-collapse:collapse">
<tr><td width="110" valign="top" style="padding:0 12pt 0 0"><img width="96" height="96" src="{logo}" alt="Example Co"></td>
<td valign="top"><p><b><span style="font-family:Calibri;font-size:12pt;color:#1F3864">Alex Example</span></b></p>
<p><span style="font-family:Calibri;font-size:10pt;color:#595959">Support lead, Example Co</span></p>
<p><span style="font-family:Calibri;font-size:10pt"><a href="https://example.org/">example.org</a></span></p></td></tr>
<tr><td colspan="2"><img width="120" height="40" src="{p1}" alt="Partner one"> <img width="120" height="40" src="{p2}" alt="Partner two"> <img width="120" height="40" src="{p3}" alt="Partner three"> <img src="https://example.org/pixel.gif" width="1" height="1"></td></tr>
</table></body></html>"#
    );
    let known = Images::default();
    let read = |_: &std::path::Path| None;
    let mut r = crate::mail::rich::images::Resolver::new(&known, &read);
    let doc = crate::mail::rich::import::lossy_with_images(&html, &mut |f| r.resolve(f))
        .unwrap_or_default();
    ("Alex Example".into(), doc, r.found)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_samples_hold_their_pictures() {
        let mut ed = Editor::new(model::Doc::default());
        let at = how_to(&mut ed).unwrap();
        assert_eq!(ed.image_offsets().len(), 2);
        assert!(ed.image_at(at).is_some());
        assert_eq!(ed.shown_images().len(), 2);
        let (_, doc, images) = signature();
        assert_eq!(
            doc.images().len(),
            4,
            "the logo and three partners, not the web one"
        );
        assert_eq!(images.len(), 4);
    }
}
