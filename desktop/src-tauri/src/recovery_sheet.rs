//! Printable recovery-code sheet for a Buzz 2SKD recovery kit.
//!
//! The PDF deliberately contains only the recovery code and public identity.
//! It never contains the password, private key, or encrypted backup artifact.

use printpdf::{
    BuiltinFont, Color, Mm, Op, PaintMode, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions,
    Point, Pt, RawImage, Rect, Rgb, TextItem, WindingOrder, XObjectTransform,
};
use qrcode::{Color as QrColor, EcLevel, QrCode};

pub const RECOVERY_SHEET_FILE_NAME: &str = "buzz-recovery-sheet.pdf";

const PAGE_WIDTH_MM: f32 = 215.9;
const PAGE_HEIGHT_MM: f32 = 279.4;
const BUZZ_WORDMARK: &[u8] = include_bytes!("../../public/landing/buzz-wordmark.png");

fn charcoal() -> Color {
    Color::Rgb(Rgb::new(35.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, None))
}

fn buzz_yellow() -> Color {
    Color::Rgb(Rgb::new(215.0 / 255.0, 215.0 / 255.0, 46.0 / 255.0, None))
}

fn cream() -> Color {
    Color::Rgb(Rgb::new(247.0 / 255.0, 247.0 / 255.0, 235.0 / 255.0, None))
}

fn muted() -> Color {
    Color::Rgb(Rgb::new(100.0 / 255.0, 94.0 / 255.0, 83.0 / 255.0, None))
}

fn light_rule() -> Color {
    Color::Rgb(Rgb::new(211.0 / 255.0, 209.0 / 255.0, 198.0 / 255.0, None))
}

fn white() -> Color {
    Color::Rgb(Rgb::new(1.0, 1.0, 1.0, None))
}

fn rect(x: f32, y: f32, width: f32, height: f32, mode: PaintMode) -> Rect {
    Rect {
        x: Mm(x).into(),
        y: Mm(y).into(),
        width: Mm(width).into(),
        height: Mm(height).into(),
        mode: Some(mode),
        winding_order: Some(WindingOrder::NonZero),
    }
}

fn fill_rect(ops: &mut Vec<Op>, x: f32, y: f32, width: f32, height: f32, color: Color) {
    ops.push(Op::SetFillColor { col: color });
    ops.push(Op::DrawRectangle {
        rectangle: rect(x, y, width, height, PaintMode::Fill),
    });
}

fn text(
    ops: &mut Vec<Op>,
    x: f32,
    y: f32,
    size: f32,
    font: BuiltinFont,
    color: Color,
    value: impl Into<String>,
) {
    ops.extend([
        Op::StartTextSection,
        Op::SetFillColor { col: color },
        Op::SetTextCursor {
            pos: Point::new(Mm(x), Mm(y)),
        },
        Op::SetFont {
            font: PdfFontHandle::Builtin(font),
            size: Pt(size),
        },
        Op::ShowText {
            items: vec![TextItem::Text(value.into())],
        },
        Op::EndTextSection,
    ]);
}

fn draw_dot_grid(ops: &mut Vec<Op>) {
    let mut y = 219.0;
    let mut row = 0usize;
    while y < 274.0 {
        let offset = if row.is_multiple_of(2) { 0.0 } else { 3.5 };
        let mut x = 143.0 + offset;
        while x < 211.0 {
            fill_rect(ops, x, y, 0.4, 0.4, light_rule());
            x += 7.0;
        }
        y += 7.0;
        row += 1;
    }
}

fn draw_qr(ops: &mut Vec<Op>, recovery_secret: &str) -> Result<(), String> {
    let code = QrCode::with_error_correction_level(recovery_secret.as_bytes(), EcLevel::Q)
        .map_err(|error| format!("encode recovery-code QR: {error}"))?;
    let quiet_modules = 4usize;
    let total_modules = code.width() + quiet_modules * 2;
    let qr_size_mm = 56.0f32;
    let module_mm = qr_size_mm / total_modules as f32;
    let qr_x = 140.0f32;
    let qr_y = 116.0f32;

    fill_rect(
        ops,
        qr_x + 3.0,
        qr_y - 3.0,
        qr_size_mm,
        qr_size_mm,
        buzz_yellow(),
    );
    fill_rect(ops, qr_x, qr_y, qr_size_mm, qr_size_mm, white());
    for y in 0..code.width() {
        for x in 0..code.width() {
            if code[(x, y)] == QrColor::Dark {
                let module_x = qr_x + (x + quiet_modules) as f32 * module_mm;
                let module_y = qr_y + (code.width() - 1 - y + quiet_modules) as f32 * module_mm;
                fill_rect(ops, module_x, module_y, module_mm, module_mm, charcoal());
            }
        }
    }
    Ok(())
}

fn split_npub(npub: &str) -> (&str, &str) {
    let split_at = npub.len().min(32);
    npub.split_at(split_at)
}

/// Render a one-page, print-ready recovery sheet.
pub(crate) fn render_recovery_sheet(
    recovery_secret: &str,
    npub: &str,
    created_date: &str,
) -> Result<Vec<u8>, String> {
    crate::two_skd::validate_recovery_secret(recovery_secret)?;
    if !npub.starts_with("npub1") || npub.len() < 20 {
        return Err("invalid recovery-sheet identity".to_string());
    }

    let mut document = PdfDocument::new("Buzz recovery sheet");
    let mut ops = Vec::new();

    fill_rect(&mut ops, 0.0, 0.0, PAGE_WIDTH_MM, PAGE_HEIGHT_MM, white());
    fill_rect(&mut ops, 0.0, 278.2, PAGE_WIDTH_MM, 1.2, buzz_yellow());
    draw_dot_grid(&mut ops);

    let mut warnings = Vec::new();
    let wordmark = RawImage::decode_from_bytes(BUZZ_WORDMARK, &mut warnings)
        .map_err(|error| format!("decode Buzz wordmark: {error}"))?;
    let wordmark_id = document.add_image(&wordmark);
    ops.push(Op::UseXobject {
        id: wordmark_id,
        transform: XObjectTransform {
            translate_x: Some(Mm(18.0).into()),
            translate_y: Some(Mm(245.0).into()),
            scale_x: Some(0.72),
            scale_y: Some(0.72),
            dpi: Some(300.0),
            ..Default::default()
        },
    });

    text(
        &mut ops,
        18.0,
        220.0,
        30.0,
        BuiltinFont::HelveticaBold,
        charcoal(),
        "Your Buzz recovery code.",
    );
    fill_rect(&mut ops, 18.0, 204.0, 34.0, 1.5, buzz_yellow());

    fill_rect(&mut ops, 18.0, 184.0, 179.9, 0.45, charcoal());
    fill_rect(&mut ops, 18.0, 96.0, 179.9, 0.45, light_rule());
    text(
        &mut ops,
        18.0,
        172.0,
        13.0,
        BuiltinFont::HelveticaBold,
        charcoal(),
        "Scan or type the code.",
    );
    fill_rect(&mut ops, 18.0, 149.0, 110.0, 16.0, cream());
    fill_rect(&mut ops, 18.0, 149.0, 3.0, 16.0, buzz_yellow());
    text(
        &mut ops,
        23.0,
        155.0,
        8.5,
        BuiltinFont::CourierBold,
        charcoal(),
        recovery_secret,
    );

    text(
        &mut ops,
        18.0,
        136.0,
        7.0,
        BuiltinFont::HelveticaBold,
        muted(),
        "BUZZ IDENTITY",
    );
    let (npub_first, npub_second) = split_npub(npub);
    text(
        &mut ops,
        18.0,
        129.5,
        7.5,
        BuiltinFont::Courier,
        charcoal(),
        npub_first,
    );
    text(
        &mut ops,
        18.0,
        124.5,
        7.5,
        BuiltinFont::Courier,
        charcoal(),
        npub_second,
    );
    text(
        &mut ops,
        18.0,
        106.0,
        7.0,
        BuiltinFont::HelveticaBold,
        muted(),
        format!("CREATED {created_date}"),
    );

    draw_qr(&mut ops, recovery_secret)?;
    fill_rect(&mut ops, 18.0, 39.0, 179.9, 47.0, cream());
    fill_rect(&mut ops, 18.0, 39.0, 3.0, 47.0, buzz_yellow());
    text(
        &mut ops,
        25.0,
        76.0,
        10.0,
        BuiltinFont::Helvetica,
        charcoal(),
        "Keep identity.buzzbackup somewhere you can reach if this computer is lost,",
    );
    text(
        &mut ops,
        25.0,
        66.0,
        10.0,
        BuiltinFont::Helvetica,
        charcoal(),
        "such as a USB drive or cloud storage.",
    );
    text(
        &mut ops,
        25.0,
        53.0,
        10.0,
        BuiltinFont::Helvetica,
        charcoal(),
        "On a new computer, install Buzz and choose \"Restore from a backup file.\"",
    );
    text(
        &mut ops,
        25.0,
        43.0,
        10.0,
        BuiltinFont::Helvetica,
        charcoal(),
        "Select identity.buzzbackup, enter your password, then use the code above.",
    );

    text(
        &mut ops,
        18.0,
        15.0,
        7.0,
        BuiltinFont::HelveticaBold,
        muted(),
        "buzz.xyz",
    );

    let page = PdfPage::new(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), ops);
    let bytes = document
        .with_pages(vec![page])
        .save(&PdfSaveOptions::default(), &mut warnings);
    if !bytes.starts_with(b"%PDF-") {
        return Err("render recovery sheet: invalid PDF output".to_string());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use printpdf::{PdfDocument, PdfParseOptions};

    const RECOVERY_CODE: &str = "buzz-recovery-v1-00112233445566778899aabbccddeeff";
    const NPUB: &str = "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5hq3g5";

    #[test]
    fn renders_a_single_page_with_recovery_material_and_no_backup() {
        let bytes = render_recovery_sheet(RECOVERY_CODE, NPUB, "August 21, 2026").unwrap();
        let mut warnings = Vec::new();
        let parsed = PdfDocument::parse(
            &bytes,
            &PdfParseOptions {
                fail_on_error: true,
            },
            &mut warnings,
        )
        .unwrap();
        let text = parsed
            .extract_text()
            .into_iter()
            .flatten()
            .collect::<String>();

        assert_eq!(parsed.pages.len(), 1);
        assert!(text.contains(RECOVERY_CODE));
        assert!(text.contains("Your Buzz recovery code."));
        assert!(text.contains("if this computer is lost"));
        assert!(text.contains("USB drive or cloud storage"));
        assert!(text.contains("On a new computer"));
        assert!(text.contains("Restore from a backup file."));
        assert!(text.contains("Select identity.buzzbackup"));
        assert!(text.contains(NPUB));
        assert!(!text.contains("buzz2skd1:"));
        assert!(!text.contains("password="));

        if let Ok(path) = std::env::var("BUZZ_RECOVERY_SHEET_PREVIEW") {
            std::fs::write(path, bytes).unwrap();
        }
    }

    #[test]
    fn rejects_malformed_recovery_material() {
        assert!(render_recovery_sheet("not-a-code", NPUB, "August 21, 2026").is_err());
        assert!(render_recovery_sheet(RECOVERY_CODE, "not-an-npub", "August 21, 2026").is_err());
    }
}
