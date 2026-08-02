use super::manifest::{ArchiveKind, Artifact, License, Limits};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

const OWNERSHIP_MARKER: &str = ".buzz-guardian-extraction-owner";

pub(crate) fn inspect_and_extract<R: Read + Seek>(
    mut reader: R,
    destination: &Path,
    artifact: &Artifact,
    license: &License,
    limits: Limits,
) -> Result<(), String> {
    verify_archive(&mut reader, artifact)?;
    let parent_path = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .ok_or("extraction destination must have a final path component")?;
    let parent = Dir::open_ambient_dir(parent_path, ambient_authority())
        .map_err(|e| format!("open extraction parent: {e}"))?;
    parent
        .create_dir(name)
        .map_err(|e| format!("create extraction directory: {e}"))?;
    let extraction_dir = match parent.open_dir(name) {
        Ok(directory) => directory,
        Err(error) => {
            let _ = parent.remove_dir(name);
            return Err(format!("open extraction directory: {error}"));
        }
    };
    let ownership_token = uuid::Uuid::new_v4().simple().to_string();
    let mut marker_options = OpenOptions::new();
    marker_options.write(true).create_new(true);
    if let Err(error) = extraction_dir
        .open_with(OWNERSHIP_MARKER, &marker_options)
        .and_then(|mut marker| marker.write_all(ownership_token.as_bytes()))
    {
        drop(extraction_dir);
        let _ = parent.remove_dir(name);
        return Err(format!("create extraction ownership marker: {error}"));
    }
    let result = match &artifact.archive_kind {
        ArchiveKind::TarGz => extract_tar(reader, &extraction_dir, &artifact.binary_path, limits),
        ArchiveKind::Zip => extract_zip(reader, &extraction_dir, &artifact.binary_path, limits),
    }
    .and_then(|expanded_size| {
        if expanded_size != artifact.expanded_size {
            return Err("archive expanded size does not match manifest".into());
        }
        verify_file(
            &extraction_dir,
            &artifact.binary_path,
            artifact.binary_size,
            &artifact.binary_sha256,
        )?;
        verify_file_digest(&extraction_dir, &license.path, &license.sha256)?;
        verify_file_digest(
            &extraction_dir,
            &license.notice_path,
            &license.notice_sha256,
        )?;
        let binary = extraction_dir
            .open(safe_path(&artifact.binary_path)?)
            .map_err(|e| e.to_string())?;
        verify_binary_format(binary, &artifact.os, &artifact.arch)
    });
    if let Err(error) = result {
        cleanup_failed_extraction(&extraction_dir);
        let owns_visible_destination =
            visible_destination_has_token(&parent, name, &ownership_token);
        let _ = extraction_dir.remove_file(OWNERSHIP_MARKER);
        drop(extraction_dir);
        if owns_visible_destination {
            let _ = parent.remove_dir(name);
        }
        return Err(error);
    }
    extraction_dir
        .remove_file(OWNERSHIP_MARKER)
        .map_err(|e| format!("remove extraction ownership marker: {e}"))?;
    Ok(())
}

fn cleanup_failed_extraction(root: &Dir) {
    for path in [
        "numbat",
        "numbat.exe",
        "LICENSE",
        "THIRD_PARTY_LICENSES.txt",
        "README.md",
        "SECURITY.md",
        "CONTRIBUTING.md",
        "docs",
        "rules",
    ] {
        let _ = root.remove_file(path);
        let _ = root.remove_dir_all(path);
    }
}

fn visible_destination_has_token(parent: &Dir, name: &std::ffi::OsStr, expected: &str) -> bool {
    let Ok(directory) = parent.open_dir(name) else {
        return false;
    };
    let Ok(mut marker) = directory.open(OWNERSHIP_MARKER) else {
        return false;
    };
    let mut token = String::new();
    marker.read_to_string(&mut token).is_ok() && token == expected
}

fn verify_archive<R: Read + Seek>(reader: &mut R, artifact: &Artifact) -> Result<(), String> {
    let length = reader.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    if length != artifact.archive_size {
        reader.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        return Err("archive size does not match manifest".into());
    }
    reader.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    reader.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    if hex::encode(hash.finalize()) != artifact.archive_sha256 {
        return Err("archive digest does not match manifest".into());
    }
    Ok(())
}

fn safe_path(name: &str) -> Result<PathBuf, String> {
    if name.contains('\\') || name.is_empty() || !name.is_ascii() || name.contains('\0') {
        return Err("unsafe archive path".into());
    }
    let path = Path::new(name);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("unsafe archive path".into());
    }
    Ok(path.to_owned())
}

fn record_entry(
    seen: &mut HashSet<String>,
    name: &str,
    count: &mut u64,
    expected_binary: &str,
    limits: Limits,
) -> Result<PathBuf, String> {
    *count += 1;
    if *count > limits.max_entries {
        return Err("archive entry-count limit exceeded".into());
    }
    let path = safe_path(name.trim_end_matches('/'))?;
    if !approved_archive_path(&path, expected_binary) {
        return Err("archive contains an unapproved path".into());
    }
    let folded = name.trim_end_matches('/').to_ascii_lowercase();
    if !seen.insert(folded) {
        return Err("duplicate or case-colliding archive path".into());
    }
    Ok(path)
}

fn approved_archive_path(path: &Path, expected_binary: &str) -> bool {
    let value = path.to_str().unwrap_or_default();
    matches!(
        value,
        "LICENSE"
            | "THIRD_PARTY_LICENSES.txt"
            | "README.md"
            | "SECURITY.md"
            | "CONTRIBUTING.md"
            | "docs"
            | "rules"
    ) || value == expected_binary
        || value.starts_with("docs/")
        || value.starts_with("rules/")
}

fn copy_file<R: Read>(
    source: R,
    root: &Dir,
    path: &Path,
    declared: u64,
    total: &mut u64,
    limits: Limits,
) -> Result<(), String> {
    if declared > limits.max_single_file_size {
        return Err("archive file-size limit exceeded".into());
    }
    *total = total
        .checked_add(declared)
        .ok_or("expanded size overflow")?;
    if *total > limits.max_expanded_size {
        return Err("expanded-size limit exceeded".into());
    }
    if let Some(parent) = path.parent() {
        root.create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = root
        .open_with(path, &options)
        .map_err(|e| format!("create extracted file: {e}"))?;
    let copied = io::copy(&mut source.take(declared + 1), &mut file).map_err(|e| e.to_string())?;
    if copied != declared {
        return Err("archive entry length mismatch".into());
    }
    Ok(())
}

fn extract_tar<R: Read + Seek>(
    mut reader: R,
    destination: &Dir,
    expected_binary: &str,
    limits: Limits,
) -> Result<u64, String> {
    let archive_size = stream_len(&mut reader)?;
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);
    let mut seen = HashSet::new();
    let mut count = 0;
    let mut total = 0;
    for item in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = item.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_str()
            .ok_or("non-UTF-8 archive path")?
            .to_owned();
        let relative = record_entry(&mut seen, &name, &mut count, expected_binary, limits)?;
        if kind.is_dir() {
            destination
                .create_dir_all(relative)
                .map_err(|e| e.to_string())?;
        } else if kind.is_file() {
            let size = entry.header().size().map_err(|e| e.to_string())?;
            copy_file(&mut entry, destination, &relative, size, &mut total, limits)?;
        } else {
            return Err("links and special archive entries are forbidden".into());
        }
    }
    if archive_size == 0
        || total
            > archive_size
                .checked_mul(limits.max_compression_ratio)
                .ok_or("compression-ratio limit overflow")?
    {
        return Err("archive compression-ratio limit exceeded".into());
    }
    Ok(total)
}

fn extract_zip<R: Read + Seek>(
    mut reader: R,
    destination: &Dir,
    expected_binary: &str,
    limits: Limits,
) -> Result<u64, String> {
    let archive_size = stream_len(&mut reader)?;
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    let mut seen = HashSet::new();
    let mut count = 0;
    let mut total = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let relative = record_entry(&mut seen, entry.name(), &mut count, expected_binary, limits)?;
        if entry.is_dir() {
            destination
                .create_dir_all(relative)
                .map_err(|e| e.to_string())?;
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
        {
            return Err("links and special archive entries are forbidden".into());
        }
        let size = entry.size();
        copy_file(&mut entry, destination, &relative, size, &mut total, limits)?;
    }
    if archive_size == 0
        || total
            > archive_size
                .checked_mul(limits.max_compression_ratio)
                .ok_or("compression-ratio limit overflow")?
    {
        return Err("archive compression-ratio limit exceeded".into());
    }
    Ok(total)
}

fn verify_file(root: &Dir, relative: &str, size: u64, expected: &str) -> Result<(), String> {
    let path = safe_path(relative)?;
    let metadata = root
        .symlink_metadata(&path)
        .map_err(|_| format!("missing required file {relative}"))?;
    if !metadata.file_type().is_file() || metadata.len() != size {
        return Err(format!("required file size mismatch: {relative}"));
    }
    let mut file = root.open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if hex::encode(hash.finalize()) != expected {
        return Err(format!("required file digest mismatch: {relative}"));
    }
    Ok(())
}

fn stream_len<R: Seek>(reader: &mut R) -> Result<u64, String> {
    let position = reader.stream_position().map_err(|e| e.to_string())?;
    let length = reader.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    reader
        .seek(SeekFrom::Start(position))
        .map_err(|e| e.to_string())?;
    Ok(length)
}

fn verify_file_digest(root: &Dir, relative: &str, expected: &str) -> Result<(), String> {
    let size = root
        .symlink_metadata(safe_path(relative)?)
        .map_err(|_| format!("missing required file {relative}"))?
        .len();
    verify_file(root, relative, size, expected)
}

fn verify_binary_format<R: Read + Seek>(mut file: R, os: &str, arch: &str) -> Result<(), String> {
    let mut head = [0u8; 64];
    file.read_exact(&mut head)
        .map_err(|_| "binary header is truncated")?;
    let valid = match os {
        "linux" => {
            head[..4] == [0x7f, b'E', b'L', b'F']
                && head[4] == 2
                && head[5] == 1
                && u16::from_le_bytes([head[18], head[19]])
                    == if arch == "amd64" { 62 } else { 183 }
        }
        "windows" => {
            let pe_offset = u32::from_le_bytes(head[60..64].try_into().unwrap()) as u64;
            let mut pe = [0u8; 6];
            head[..2] == *b"MZ"
                && pe_offset >= 64
                && file.seek(SeekFrom::Start(pe_offset)).is_ok()
                && file.read_exact(&mut pe).is_ok()
                && pe[..4] == *b"PE\0\0"
                && u16::from_le_bytes([pe[4], pe[5]])
                    == if arch == "amd64" { 0x8664 } else { 0xaa64 }
        }
        "darwin" => {
            head[..4] == [0xcf, 0xfa, 0xed, 0xfe]
                && u32::from_le_bytes(head[4..8].try_into().unwrap())
                    == if arch == "amd64" {
                        0x01000007
                    } else {
                        0x0100000c
                    }
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("binary format does not match authorized target".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::io::{Cursor, Write};
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn limits() -> Limits {
        Limits {
            max_entries: 8,
            max_expanded_size: 1024,
            max_single_file_size: 512,
            max_compression_ratio: 20,
        }
    }

    fn cap_dir(path: &Path) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    fn zip_with_files(files: &[(&str, &[u8])]) -> Cursor<Vec<u8>> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap()
    }

    fn tar_with_symlink() -> Cursor<Vec<u8>> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../../escape").unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "numbat", Cursor::new(Vec::<u8>::new()))
            .unwrap();
        let encoder = builder.into_inner().unwrap();
        Cursor::new(encoder.finish().unwrap())
    }

    fn tar_with_files(files: &[(&str, &[u8])]) -> Cursor<Vec<u8>> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, *name, Cursor::new(*body))
                .unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        Cursor::new(encoder.finish().unwrap())
    }

    fn linux_binary() -> Vec<u8> {
        let mut binary = vec![0u8; 64];
        binary[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
        binary[18..20].copy_from_slice(&62u16.to_le_bytes());
        binary
    }

    fn digest(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn fixture(kind: ArchiveKind) -> (Cursor<Vec<u8>>, Artifact, License) {
        let binary = linux_binary();
        let license_body = b"Apache-2.0";
        let notice_body = b"third-party notices";
        let files = [
            ("numbat", binary.as_slice()),
            ("LICENSE", license_body.as_slice()),
            ("THIRD_PARTY_LICENSES.txt", notice_body.as_slice()),
        ];
        let archive = match &kind {
            ArchiveKind::Zip => zip_with_files(&files),
            ArchiveKind::TarGz => tar_with_files(&files),
        };
        let artifact = Artifact {
            os: "linux".into(),
            arch: "amd64".into(),
            archive_kind: kind,
            asset_name: "fixture".into(),
            url: "https://example.invalid/fixture".into(),
            archive_sha256: digest(archive.get_ref()),
            archive_size: archive.get_ref().len() as u64,
            expanded_size: (binary.len() + license_body.len() + notice_body.len()) as u64,
            binary_path: "numbat".into(),
            binary_sha256: digest(&binary),
            binary_size: binary.len() as u64,
        };
        let license = License {
            spdx: "Apache-2.0".into(),
            path: "LICENSE".into(),
            sha256: digest(license_body),
            notice_path: "THIRD_PARTY_LICENSES.txt".into(),
            notice_sha256: digest(notice_body),
        };
        (archive, artifact, license)
    }

    fn assert_failure_cleans(
        archive: Cursor<Vec<u8>>,
        artifact: &Artifact,
        license: &License,
        limits: Limits,
        expected: &str,
    ) {
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("extract");
        let error =
            inspect_and_extract(archive, &destination, artifact, license, limits).unwrap_err();
        assert!(error.contains(expected), "unexpected error: {error}");
        assert!(
            !destination.exists(),
            "failed extraction was not cleaned up"
        );
    }

    #[test]
    fn rejects_paths_that_can_escape_or_confuse_platforms() {
        for path in [
            "../numbat",
            "/numbat",
            "a/../../numbat",
            "a\\numbat",
            "rules/café",
            "",
        ] {
            assert!(safe_path(path).is_err(), "accepted unsafe path {path:?}");
        }
        assert!(safe_path("rules/approved.yaml").is_ok());
    }

    #[test]
    fn rejects_unapproved_archive_content() {
        let archive = zip_with_files(&[("surprise.sh", b"nope")]);
        let root = tempfile::tempdir().unwrap();
        assert!(
            extract_zip(archive, &cap_dir(root.path()), "numbat", limits())
                .unwrap_err()
                .contains("unapproved path")
        );

        let archive = zip_with_files(&[("numbat.exe", b"wrong target")]);
        let root = tempfile::tempdir().unwrap();
        assert!(
            extract_zip(archive, &cap_dir(root.path()), "numbat", limits())
                .unwrap_err()
                .contains("unapproved path")
        );
    }

    #[test]
    fn rejects_duplicate_and_case_colliding_zip_entries() {
        let archive = zip_with_files(&[("rules/x", b"one"), ("rules/X", b"two")]);
        let root = tempfile::tempdir().unwrap();
        assert!(
            extract_zip(archive, &cap_dir(root.path()), "numbat", limits())
                .unwrap_err()
                .contains("case-colliding")
        );
    }

    #[test]
    fn rejects_zip_links() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink("numbat", "../../escape", SimpleFileOptions::default())
            .unwrap();
        let archive = writer.finish().unwrap();
        let root = tempfile::tempdir().unwrap();
        assert!(
            extract_zip(archive, &cap_dir(root.path()), "numbat", limits())
                .unwrap_err()
                .contains("special archive entries")
        );
    }

    #[test]
    fn rejects_tar_links() {
        let root = tempfile::tempdir().unwrap();
        assert!(extract_tar(
            tar_with_symlink(),
            &cap_dir(root.path()),
            "numbat",
            limits(),
        )
        .unwrap_err()
        .contains("special archive entries"));
    }

    #[test]
    fn rejects_entry_count_and_single_file_bombs() {
        let archive = zip_with_files(&[("rules/one", b"1"), ("rules/two", b"2")]);
        let root = tempfile::tempdir().unwrap();
        let mut strict = limits();
        strict.max_entries = 1;
        assert!(
            extract_zip(archive, &cap_dir(root.path()), "numbat", strict)
                .unwrap_err()
                .contains("entry-count")
        );

        let archive = zip_with_files(&[("rules/large", &[0u8; 513])]);
        let root = tempfile::tempdir().unwrap();
        assert!(
            extract_zip(archive, &cap_dir(root.path()), "numbat", limits())
                .unwrap_err()
                .contains("file-size")
        );
    }

    #[test]
    fn rejects_expanded_size_and_compression_ratio_bombs() {
        let archive = zip_with_files(&[("rules/one", &[0u8; 500]), ("rules/two", &[0u8; 500])]);
        let root = tempfile::tempdir().unwrap();
        let mut strict = limits();
        strict.max_expanded_size = 900;
        assert!(
            extract_zip(archive, &cap_dir(root.path()), "numbat", strict)
                .unwrap_err()
                .contains("expanded-size")
        );

        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("rules/bomb", options).unwrap();
        writer.write_all(&[0u8; 1000]).unwrap();
        let archive = writer.finish().unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut strict = limits();
        strict.max_single_file_size = 1024;
        strict.max_compression_ratio = 1;
        assert!(
            extract_zip(archive, &cap_dir(root.path()), "numbat", strict)
                .unwrap_err()
                .contains("compression-ratio")
        );
    }

    #[test]
    fn public_flow_extracts_zip_and_tar() {
        for kind in [ArchiveKind::Zip, ArchiveKind::TarGz] {
            let (archive, artifact, license) = fixture(kind);
            let parent = tempfile::tempdir().unwrap();
            let destination = parent.path().join("extract");
            inspect_and_extract(archive, &destination, &artifact, &license, limits()).unwrap();
            assert_eq!(
                fs::read(destination.join("numbat")).unwrap(),
                linux_binary()
            );
            assert_eq!(
                fs::read(destination.join("LICENSE")).unwrap(),
                b"Apache-2.0"
            );
            assert_eq!(
                fs::read(destination.join("THIRD_PARTY_LICENSES.txt")).unwrap(),
                b"third-party notices"
            );
        }
    }

    #[test]
    fn public_flow_rejects_archive_size_and_digest_before_creating_destination() {
        let (archive, mut artifact, license) = fixture(ArchiveKind::Zip);
        artifact.archive_size += 1;
        assert_failure_cleans(archive, &artifact, &license, limits(), "archive size");

        let (archive, mut artifact, license) = fixture(ArchiveKind::TarGz);
        artifact.archive_sha256 = "0".repeat(64);
        assert_failure_cleans(archive, &artifact, &license, limits(), "archive digest");
    }

    #[test]
    fn public_flow_rejects_manifest_mismatches_and_cleans_destination() {
        let (archive, mut artifact, license) = fixture(ArchiveKind::Zip);
        artifact.expanded_size += 1;
        assert_failure_cleans(archive, &artifact, &license, limits(), "expanded size");

        let (archive, mut artifact, license) = fixture(ArchiveKind::Zip);
        artifact.binary_size += 1;
        assert_failure_cleans(archive, &artifact, &license, limits(), "file size mismatch");

        let (archive, mut artifact, license) = fixture(ArchiveKind::TarGz);
        artifact.binary_sha256 = "0".repeat(64);
        assert_failure_cleans(
            archive,
            &artifact,
            &license,
            limits(),
            "file digest mismatch",
        );

        let (archive, mut artifact, license) = fixture(ArchiveKind::TarGz);
        artifact.binary_path = "rules/missing".into();
        assert_failure_cleans(archive, &artifact, &license, limits(), "unapproved path");

        let (archive, mut artifact, license) = fixture(ArchiveKind::Zip);
        artifact.arch = "arm64".into();
        assert_failure_cleans(archive, &artifact, &license, limits(), "authorized target");

        let (archive, artifact, mut license) = fixture(ArchiveKind::Zip);
        license.notice_sha256 = "0".repeat(64);
        assert_failure_cleans(
            archive,
            &artifact,
            &license,
            limits(),
            "file digest mismatch",
        );
    }

    #[test]
    fn public_flow_applies_limits_and_cleans_destination() {
        let (archive, artifact, license) = fixture(ArchiveKind::Zip);
        let mut strict = limits();
        strict.max_entries = 2;
        assert_failure_cleans(archive, &artifact, &license, strict, "entry-count");

        let (archive, artifact, license) = fixture(ArchiveKind::TarGz);
        let mut strict = limits();
        strict.max_single_file_size = 63;
        assert_failure_cleans(archive, &artifact, &license, strict, "file-size");

        let (archive, artifact, license) = fixture(ArchiveKind::Zip);
        let mut strict = limits();
        strict.max_expanded_size = artifact.expanded_size - 1;
        assert_failure_cleans(archive, &artifact, &license, strict, "expanded-size");

        let (archive, artifact, license) = fixture(ArchiveKind::TarGz);
        let mut strict = limits();
        strict.max_compression_ratio = 0;
        assert_failure_cleans(archive, &artifact, &license, strict, "compression-ratio");
    }

    #[test]
    fn public_flow_refuses_an_existing_destination() {
        let (archive, artifact, license) = fixture(ArchiveKind::Zip);
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("extract");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), b"keep").unwrap();
        let error =
            inspect_and_extract(archive, &destination, &artifact, &license, limits()).unwrap_err();
        assert!(error.contains("create extraction directory"));
        assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn held_directory_capability_defeats_destination_symlink_swap() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("extract");
        let held_location = parent.path().join("held");
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(&destination).unwrap();
        let capability = cap_dir(&destination);
        fs::rename(&destination, &held_location).unwrap();
        symlink(outside.path(), &destination).unwrap();

        let archive = zip_with_files(&[("rules/proof", b"confined")]);
        extract_zip(archive, &capability, "numbat", limits()).unwrap();

        assert_eq!(
            fs::read(held_location.join("rules/proof")).unwrap(),
            b"confined"
        );
        assert!(!outside.path().join("rules/proof").exists());
    }
}
