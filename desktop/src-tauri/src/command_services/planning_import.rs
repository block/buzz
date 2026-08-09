use calamine::{open_workbook_auto_from_rs, Data, Reader};
use quick_xml::{events::Event, Reader as XmlReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Seek},
    path::Path,
};
use zip::ZipArchive;

use super::apple_inputs::extract_planning_pdf;

pub(crate) const MAXIMUM_DOCUMENT_BYTES: u64 = 50 * 1024 * 1024;
const MAXIMUM_EXTRACTED_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_BLOCKS: usize = 20_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ExtractedPlanningBlock {
    TableRow {
        location: String,
        cells: Vec<String>,
    },
    SpreadsheetCell {
        location: String,
        sheet: String,
        coordinate: String,
        value: String,
    },
    SpreadsheetMerge {
        location: String,
        sheet: String,
        range: String,
    },
    PdfPage {
        location: String,
        page: usize,
        text: String,
        confidence: Option<f64>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExtractedSheet {
    pub(crate) name: String,
    pub(crate) maximum_row: usize,
    pub(crate) maximum_column: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExtractedPlanningDocument {
    pub(crate) filename: String,
    pub(crate) extension: String,
    pub(crate) sha256: String,
    pub(crate) size_bytes: u64,
    pub(crate) blocks: Vec<ExtractedPlanningBlock>,
    pub(crate) pages: Vec<usize>,
    pub(crate) sheets: Vec<ExtractedSheet>,
    pub(crate) truncated: bool,
}

pub(crate) fn extract_planning_document(path: &Path) -> Result<ExtractedPlanningDocument, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot inspect document: {error}"))?;
    if !metadata.is_file() {
        return Err("selected planning document is not a regular file".into());
    }
    if metadata.len() > MAXIMUM_DOCUMENT_BYTES {
        return Err("planning documents are limited to 50 MiB".into());
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "planning document filename is invalid".to_string())?
        .to_string();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "planning document must be DOCX, XLSX, or PDF".to_string())?;
    if !matches!(extension.as_str(), "docx" | "xlsx" | "pdf") {
        return Err("planning document must be DOCX, XLSX, or PDF".into());
    }
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read planning document: {error}"))?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let (blocks, pages, sheets) = match extension.as_str() {
        "docx" => (extract_docx(&bytes)?, Vec::new(), Vec::new()),
        "xlsx" => {
            let (blocks, sheets) = extract_xlsx(&bytes)?;
            (blocks, Vec::new(), sheets)
        }
        "pdf" => {
            let pages = extract_planning_pdf(path)?;
            let page_numbers = pages.iter().map(|page| page.page).collect();
            let blocks = pages
                .into_iter()
                .map(|page| ExtractedPlanningBlock::PdfPage {
                    location: format!("page {}", page.page),
                    page: page.page,
                    text: page.text,
                    confidence: page.confidence,
                })
                .collect();
            (blocks, page_numbers, Vec::new())
        }
        _ => unreachable!(),
    };
    let document = ExtractedPlanningDocument {
        filename,
        extension,
        sha256,
        size_bytes: metadata.len(),
        blocks,
        pages,
        sheets,
        truncated: false,
    };
    ensure_output_bound(&document)?;
    Ok(document)
}

fn extract_docx(bytes: &[u8]) -> Result<Vec<ExtractedPlanningBlock>, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|error| format!("invalid DOCX: {error}"))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|_| "DOCX is missing word/document.xml".to_string())?
        .read_to_string(&mut xml)
        .map_err(|error| format!("cannot read DOCX document XML: {error}"))?;

    let mut reader = XmlReader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut table_index = 0usize;
    let mut row_index = 0usize;
    let mut in_row = false;
    let mut in_cell = false;
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut blocks = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => match start.local_name().as_ref() {
                b"tbl" => {
                    table_index += 1;
                    row_index = 0;
                }
                b"tr" => {
                    in_row = true;
                    row_index += 1;
                    cells.clear();
                }
                b"tc" if in_row => {
                    in_cell = true;
                    cell.clear();
                }
                _ => {}
            },
            Ok(Event::Text(text)) if in_cell => {
                let decoded = text
                    .decode()
                    .map_err(|error| format!("invalid DOCX text: {error}"))?;
                if !cell.is_empty() && !decoded.is_empty() {
                    cell.push(' ');
                }
                cell.push_str(&decoded);
            }
            Ok(Event::End(end)) => match end.local_name().as_ref() {
                b"tc" if in_cell => {
                    cells.push(cell.trim().to_string());
                    in_cell = false;
                }
                b"tr" if in_row => {
                    if !cells.iter().all(String::is_empty) {
                        blocks.push(ExtractedPlanningBlock::TableRow {
                            location: format!("table {table_index} row {row_index}"),
                            cells: cells.clone(),
                        });
                        enforce_block_count(&blocks)?;
                    }
                    in_row = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("invalid DOCX XML: {error}")),
            _ => {}
        }
    }
    Ok(blocks)
}

fn extract_xlsx(
    bytes: &[u8],
) -> Result<(Vec<ExtractedPlanningBlock>, Vec<ExtractedSheet>), String> {
    let mut workbook = open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|error| format!("invalid XLSX: {error}"))?;
    let sheet_names = workbook.sheet_names();
    let merges = extract_xlsx_merges(bytes, sheet_names.len())?;
    let mut blocks = Vec::new();
    let mut sheets = Vec::new();
    for (index, name) in sheet_names.into_iter().enumerate() {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|error| format!("cannot read XLSX sheet {name}: {error}"))?;
        let mut maximum_row = 0usize;
        let mut maximum_column = 0usize;
        for (row, column, value) in range.cells() {
            if matches!(value, Data::Empty) {
                continue;
            }
            let coordinate = cell_coordinate(row, column);
            blocks.push(ExtractedPlanningBlock::SpreadsheetCell {
                location: format!("{name}!{coordinate}"),
                sheet: name.clone(),
                coordinate,
                value: value.to_string(),
            });
            maximum_row = maximum_row.max(row + 1);
            maximum_column = maximum_column.max(column + 1);
            enforce_block_count(&blocks)?;
        }
        for merged in merges.get(index).into_iter().flatten() {
            blocks.push(ExtractedPlanningBlock::SpreadsheetMerge {
                location: format!("{name}!{merged}"),
                sheet: name.clone(),
                range: merged.clone(),
            });
            enforce_block_count(&blocks)?;
        }
        sheets.push(ExtractedSheet {
            name,
            maximum_row,
            maximum_column,
        });
    }
    Ok((blocks, sheets))
}

fn extract_xlsx_merges(bytes: &[u8], sheet_count: usize) -> Result<Vec<Vec<String>>, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|error| format!("invalid XLSX: {error}"))?;
    let mut all = Vec::with_capacity(sheet_count);
    for index in 1..=sheet_count {
        let mut xml = String::new();
        let path = format!("xl/worksheets/sheet{index}.xml");
        if let Ok(mut sheet) = archive.by_name(&path) {
            sheet
                .read_to_string(&mut xml)
                .map_err(|error| format!("cannot read XLSX worksheet XML: {error}"))?;
        }
        let mut reader = XmlReader::from_str(&xml);
        let mut ranges = Vec::new();
        loop {
            match reader.read_event() {
                Ok(Event::Empty(element)) | Ok(Event::Start(element))
                    if element.local_name().as_ref() == b"mergeCell" =>
                {
                    for attribute in element.attributes().flatten() {
                        if attribute.key.local_name().as_ref() == b"ref" {
                            ranges.push(String::from_utf8_lossy(&attribute.value).into_owned());
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(error) => return Err(format!("invalid XLSX worksheet XML: {error}")),
                _ => {}
            }
        }
        all.push(ranges);
    }
    Ok(all)
}

fn cell_coordinate(row: usize, column: usize) -> String {
    let mut value = column + 1;
    let mut letters = String::new();
    while value > 0 {
        let remainder = (value - 1) % 26;
        letters.insert(0, (b'A' + remainder as u8) as char);
        value = (value - 1) / 26;
    }
    format!("{letters}{}", row + 1)
}

fn enforce_block_count(blocks: &[ExtractedPlanningBlock]) -> Result<(), String> {
    if blocks.len() > MAXIMUM_BLOCKS {
        Err("planning document contains more than 20,000 extracted entries".into())
    } else {
        Ok(())
    }
}

fn ensure_output_bound(document: &ExtractedPlanningDocument) -> Result<(), String> {
    let size = serde_json::to_vec(document)
        .map_err(|error| format!("cannot encode extracted document: {error}"))?
        .len();
    if size > MAXIMUM_EXTRACTED_BYTES {
        Err("planning document extraction exceeds the 4 MiB output limit".into())
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
fn _assert_cursor_bounds<T: Read + Seek>(_reader: T) {}
