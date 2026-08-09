use atomic_write_file::AtomicWriteFile;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
use zip::{write::SimpleFileOptions, ZipWriter};

use crate::commands::project_execution::ArtifactWriteResult;

const MAXIMUM_ARTIFACT_BYTES: usize = 25 * 1024 * 1024;

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn pdf_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn slug(value: &str) -> String {
    let mut result = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while result.contains("--") {
        result = result.replace("--", "-");
    }
    let trimmed = result.trim_matches('-');
    if trimmed.is_empty() {
        "command-adviser-output".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}

fn unique_path(root: &Path, stem: &str, extension: &str) -> PathBuf {
    let base = slug(stem);
    let first = root.join(format!("{base}.{extension}"));
    if !first.exists() {
        return first;
    }
    for suffix in 2..10_000 {
        let candidate = root.join(format!("{base}-{suffix}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    root.join(format!(
        "{base}-{}.{}",
        chrono::Utc::now().timestamp(),
        extension
    ))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAXIMUM_ARTIFACT_BYTES {
        return Err("Generated artefact exceeds the 25 MiB limit.".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "Cannot create output folder.".to_string())?;
    }
    let mut file =
        AtomicWriteFile::open(path).map_err(|_| "Cannot open the output file.".to_string())?;
    file.write_all(bytes)
        .map_err(|_| "Cannot write the output file.".to_string())?;
    file.commit()
        .map_err(|_| "Cannot commit the output file.".to_string())
}

pub(crate) fn pdf_bytes(title: &str, body: &str) -> Vec<u8> {
    let lines = std::iter::once(title)
        .chain(body.lines())
        .take(46)
        .map(|line| line.chars().take(105).collect::<String>())
        .collect::<Vec<_>>();
    let mut content = String::from("BT /F1 11 Tf 48 790 Td 14 TL ");
    for line in lines {
        content.push_str(&format!("({}) Tj T* ", pdf_escape(&line)));
    }
    content.push_str("ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn zip_bytes(files: &[(&str, String)]) -> Result<Vec<u8>, String> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = ZipWriter::new(cursor);
    for (name, content) in files {
        archive
            .start_file(*name, SimpleFileOptions::default())
            .map_err(|_| "Cannot create Office package.".to_string())?;
        archive
            .write_all(content.as_bytes())
            .map_err(|_| "Cannot write Office package.".to_string())?;
    }
    archive
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|_| "Cannot finish Office package.".to_string())
}

fn docx_bytes(title: &str, body: &str) -> Result<Vec<u8>, String> {
    let paragraphs = std::iter::once(title)
        .chain(body.lines())
        .map(|line| format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", xml_escape(line)))
        .collect::<String>();
    zip_bytes(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.into(),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.into(),
        ),
        (
            "word/document.xml",
            format!(r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{paragraphs}</w:body></w:document>"#),
        ),
    ])
}

fn pptx_bytes(title: &str, body: &str) -> Result<Vec<u8>, String> {
    let text = xml_escape(&format!("{title}\n{body}"));
    zip_bytes(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>"#.into(),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#.into(),
        ),
        (
            "ppt/presentation.xml",
            r#"<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId1" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></p:sldIdLst></p:presentation>"#.into(),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#.into(),
        ),
        (
            "ppt/slides/slide1.xml",
            format!(r#"<?xml version="1.0"?><p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#),
        ),
    ])
}

fn xlsx_bytes(title: &str, body: &str) -> Result<Vec<u8>, String> {
    let rows = std::iter::once(title)
        .chain(body.lines())
        .enumerate()
        .map(|(index, line)| {
            format!(
                r#"<row r="{row}"><c r="A{row}" t="inlineStr"><is><t>{}</t></is></c></row>"#,
                xml_escape(line),
                row = index + 1
            )
        })
        .collect::<String>();
    zip_bytes(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#.into(),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.into(),
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Command Adviser" sheetId="1" r:id="rId1"/></sheets></workbook>"#.into(),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.into(),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!(r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#),
        ),
    ])
}

pub(crate) fn write_artifact(
    root: &Path,
    stem: &str,
    format: &str,
    title: &str,
    body: &str,
    storage_state: &str,
) -> Result<ArtifactWriteResult, String> {
    let bytes = match format {
        "pdf" => pdf_bytes(title, body),
        "docx" => docx_bytes(title, body)?,
        "pptx" => pptx_bytes(title, body)?,
        "xlsx" => xlsx_bytes(title, body)?,
        _ => return Err("Unsupported artefact format.".into()),
    };
    let path = unique_path(root, stem, format);
    atomic_write(&path, &bytes)?;
    let digest = hex::encode(Sha256::digest(&bytes));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Generated output path is invalid.".to_string())?
        .to_string();
    Ok(ArtifactWriteResult {
        file_name,
        path: path.to_string_lossy().into_owned(),
        format: format.to_string(),
        storage_state: storage_state.to_string(),
        sha256: digest,
        size_bytes: bytes.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn generates_valid_pdf_and_collision_safe_names() {
        let root = tempfile::tempdir().expect("temp");
        let first = write_artifact(
            root.path(),
            "HOD Sync",
            "pdf",
            "HOD Sync",
            "MEO tasks",
            "icloud",
        )
        .expect("pdf");
        let second = write_artifact(
            root.path(),
            "HOD Sync",
            "pdf",
            "HOD Sync",
            "MEO tasks",
            "icloud",
        )
        .expect("second pdf");
        assert_ne!(first.path, second.path);
        let bytes = fs::read(first.path).expect("read");
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(String::from_utf8_lossy(&bytes).contains("MEO tasks"));
    }

    #[test]
    fn generates_minimal_office_packages() {
        let root = tempfile::tempdir().expect("temp");
        for (format, required) in [
            (
                "docx",
                vec!["[Content_Types].xml", "_rels/.rels", "word/document.xml"],
            ),
            (
                "pptx",
                vec![
                    "[Content_Types].xml",
                    "ppt/presentation.xml",
                    "ppt/slides/slide1.xml",
                ],
            ),
            (
                "xlsx",
                vec![
                    "[Content_Types].xml",
                    "xl/workbook.xml",
                    "xl/worksheets/sheet1.xml",
                ],
            ),
        ] {
            let result = write_artifact(root.path(), "Output", format, "Title", "Body", "icloud")
                .expect("office output");
            let file = fs::File::open(result.path).expect("open");
            let mut archive = zip::ZipArchive::new(file).expect("zip");
            for name in required {
                let mut entry = archive.by_name(name).expect("required member");
                let mut content = String::new();
                entry.read_to_string(&mut content).expect("xml");
                assert!(content.contains("xml"));
            }
        }
    }
}
