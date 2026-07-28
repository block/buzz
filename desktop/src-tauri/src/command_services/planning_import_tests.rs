use super::planning_import::{
    extract_planning_document, ExtractedPlanningBlock, MAXIMUM_DOCUMENT_BYTES,
};
use std::io::Write;
use zip::{write::SimpleFileOptions, ZipWriter};

fn write_zip(entries: &[(&str, &str)], suffix: &str) -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new().suffix(suffix).tempfile().unwrap();
    let mut archive = ZipWriter::new(file.reopen().unwrap());
    for (name, contents) in entries {
        archive
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(contents.as_bytes()).unwrap();
    }
    archive.finish().unwrap();
    file
}

#[test]
fn docx_retains_table_cell_text_and_row_order() {
    let document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
      <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
        <w:body><w:tbl>
          <w:tr><w:tc><w:p><w:r><w:t>Date</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Activity</w:t></w:r></w:p></w:tc></w:tr>
          <w:tr><w:tc><w:p><w:r><w:t>29 Jul</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Secure for sea</w:t></w:r></w:p></w:tc></w:tr>
        </w:tbl></w:body>
      </w:document>"#;
    let file = write_zip(&[("word/document.xml", document)], ".docx");

    let extracted = extract_planning_document(file.path()).unwrap();

    assert_eq!(extracted.extension, "docx");
    assert_eq!(extracted.sha256.len(), 64);
    assert!(extracted.filename.ends_with(".docx"));
    assert_eq!(
        extracted.blocks,
        vec![
            ExtractedPlanningBlock::TableRow {
                location: "table 1 row 1".into(),
                cells: vec!["Date".into(), "Activity".into()],
            },
            ExtractedPlanningBlock::TableRow {
                location: "table 1 row 2".into(),
                cells: vec!["29 Jul".into(), "Secure for sea".into()],
            },
        ]
    );
}

#[test]
fn xlsx_retains_sheet_cells_and_merged_ranges() {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
      <workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
        xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
        <sheets><sheet name="Shortcast" sheetId="1" r:id="rId1"/></sheets>
      </workbook>"#;
    let relationships = r#"<?xml version="1.0" encoding="UTF-8"?>
      <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
      </Relationships>"#;
    let sheet = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
      <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
        <sheetData><row r="1">
          <c r="A1" t="inlineStr"><is><t>Time</t></is></c>
          <c r="B1" t="inlineStr"><is><t>Event</t></is></c>
        </row><row r="2">
          <c r="A2" t="inlineStr"><is><t>0800</t></is></c>
          <c r="B2" t="inlineStr"><is><t>Navigation brief</t></is></c>
        </row></sheetData>
        <mergeCells count="1"><mergeCell ref="B2:C2"/></mergeCells>
      </worksheet>"#;
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
      <Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
        <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
        <Default Extension="xml" ContentType="application/xml"/>
        <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
        <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
      </Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
      <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
      </Relationships>"#;
    let file = write_zip(
        &[
            ("[Content_Types].xml", content_types),
            ("_rels/.rels", root_rels),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", relationships),
            ("xl/worksheets/sheet1.xml", sheet),
        ],
        ".xlsx",
    );

    let extracted = extract_planning_document(file.path()).unwrap();

    assert_eq!(extracted.sheets.len(), 1);
    assert_eq!(extracted.sheets[0].name, "Shortcast");
    assert!(extracted
        .blocks
        .contains(&ExtractedPlanningBlock::SpreadsheetCell {
            location: "Shortcast!B2".into(),
            sheet: "Shortcast".into(),
            coordinate: "B2".into(),
            value: "Navigation brief".into(),
        }));
    assert!(extracted
        .blocks
        .contains(&ExtractedPlanningBlock::SpreadsheetMerge {
            location: "Shortcast!B2:C2".into(),
            sheet: "Shortcast".into(),
            range: "B2:C2".into(),
        }));
}

#[test]
fn rejects_unsupported_and_oversized_documents() {
    let unsupported = tempfile::Builder::new().suffix(".txt").tempfile().unwrap();
    assert!(extract_planning_document(unsupported.path())
        .unwrap_err()
        .contains("DOCX, XLSX, or PDF"));

    let oversized = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
    oversized
        .as_file()
        .set_len(MAXIMUM_DOCUMENT_BYTES + 1)
        .unwrap();
    assert!(extract_planning_document(oversized.path())
        .unwrap_err()
        .contains("50 MiB"));
}
