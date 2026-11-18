//! Strict parsing for S3 version listings used by destructive cleanup.
use super::{ObjectVersionEntry, ObjectVersionKind, ObjectVersionsPage};
use crate::error::MediaError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

#[derive(Debug, Default)]
struct ListVersionFields {
    key: Option<String>,
    version_id: Option<String>,
    size: Option<u64>,
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn xml_error(error: impl std::fmt::Display) -> MediaError {
    MediaError::StorageError(error.to_string())
}

fn read_element_text(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
) -> Result<String, MediaError> {
    reader
        .read_text(start.to_end().name())
        .map(|text| text.into_owned())
        .map_err(xml_error)
}

fn skip_element(reader: &mut Reader<&[u8]>, start: &BytesStart<'_>) -> Result<(), MediaError> {
    reader
        .read_to_end(start.to_end().name())
        .map_err(xml_error)?;
    Ok(())
}

fn parse_list_version_entry(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
    kind: ObjectVersionKind,
) -> Result<ObjectVersionEntry, MediaError> {
    let mut fields = ListVersionFields::default();
    loop {
        match reader.read_event().map_err(xml_error)? {
            Event::Start(child) => match local_name(child.local_name().as_ref()) {
                b"Key" => set_once(&mut fields.key, read_element_text(reader, &child)?)?,
                b"VersionId" => {
                    set_once(&mut fields.version_id, read_element_text(reader, &child)?)?
                }
                b"Size" => {
                    let size = read_element_text(reader, &child)?;
                    set_once(&mut fields.size, size.parse::<u64>().map_err(xml_error)?)?;
                }
                _ => skip_element(reader, &child)?,
            },
            Event::Empty(child) => match local_name(child.local_name().as_ref()) {
                b"Key" => set_once(&mut fields.key, String::new())?,
                b"VersionId" => set_once(&mut fields.version_id, String::new())?,
                b"Size" => set_once(&mut fields.size, 0)?,
                _ => {}
            },
            Event::End(end) if end.name().as_ref() == start.to_end().name().as_ref() => {
                let key = fields.key.ok_or_else(|| {
                    MediaError::StorageError("ListObjectVersions entry missing Key".to_string())
                })?;
                let version_id = fields.version_id.ok_or_else(|| {
                    MediaError::StorageError(
                        "ListObjectVersions entry missing VersionId".to_string(),
                    )
                })?;
                return Ok(ObjectVersionEntry {
                    key,
                    version_id,
                    kind,
                    size: if kind == ObjectVersionKind::Object {
                        fields.size.unwrap_or(0)
                    } else {
                        0
                    },
                });
            }
            Event::Eof => {
                return Err(MediaError::StorageError(
                    "unexpected EOF inside ListObjectVersions entry".to_string(),
                ));
            }
            _ => {}
        }
    }
}

pub(super) fn parse_object_versions_page(xml: &[u8]) -> Result<ObjectVersionsPage, MediaError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut entries = Vec::new();
    let mut next_key_marker = None;
    let mut next_version_id_marker = None;
    let mut is_truncated = None;
    let mut root_seen = false;
    let mut root_closed = false;

    loop {
        match reader.read_event().map_err(xml_error)? {
            Event::Start(start) if !root_seen => {
                if start.local_name().as_ref() != b"ListVersionsResult" {
                    return Err(xml_error("expected ListVersionsResult root"));
                }
                root_seen = true;
            }
            Event::Start(start) if !root_closed => match start.local_name().as_ref() {
                b"Version" => entries.push(parse_list_version_entry(
                    &mut reader,
                    &start,
                    ObjectVersionKind::Object,
                )?),
                b"DeleteMarker" => entries.push(parse_list_version_entry(
                    &mut reader,
                    &start,
                    ObjectVersionKind::DeleteMarker,
                )?),
                b"IsTruncated" => {
                    let value = read_element_text(&mut reader, &start)?;
                    let value = match value.as_str() {
                        "true" => true,
                        "false" => false,
                        _ => return Err(xml_error("invalid ListObjectVersions IsTruncated")),
                    };
                    set_once(&mut is_truncated, value)?;
                }
                b"NextKeyMarker" => {
                    set_once(
                        &mut next_key_marker,
                        read_element_text(&mut reader, &start)?,
                    )?;
                }
                b"NextVersionIdMarker" => {
                    set_once(
                        &mut next_version_id_marker,
                        read_element_text(&mut reader, &start)?,
                    )?;
                }
                b"ListVersionsResult" => return Err(xml_error("nested ListVersionsResult root")),
                _ => skip_element(&mut reader, &start)?,
            },
            Event::Empty(start) if root_seen && !root_closed => match start.local_name().as_ref() {
                b"NextKeyMarker" => set_once(&mut next_key_marker, String::new())?,
                b"NextVersionIdMarker" => set_once(&mut next_version_id_marker, String::new())?,
                b"Version" | b"DeleteMarker" | b"IsTruncated" | b"ListVersionsResult" => {
                    return Err(xml_error("empty ListObjectVersions element"));
                }
                _ => {}
            },
            Event::End(end)
                if root_seen
                    && !root_closed
                    && end.local_name().as_ref() == b"ListVersionsResult" =>
            {
                root_closed = true;
            }
            Event::Decl(_) if !root_seen => {}
            Event::Comment(_) | Event::PI(_) => {}
            Event::Text(text) if text.as_ref().iter().all(u8::is_ascii_whitespace) => {}
            Event::Eof if root_closed => break,
            Event::Eof => return Err(xml_error("incomplete ListObjectVersions document")),
            _ => {
                return Err(xml_error(
                    "unexpected content in ListObjectVersions document",
                ))
            }
        }
    }
    let is_truncated =
        is_truncated.ok_or_else(|| xml_error("ListObjectVersions missing IsTruncated"))?;

    Ok(ObjectVersionsPage {
        entries,
        next_key_marker,
        next_version_id_marker,
        is_truncated,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<(), MediaError> {
    if slot.is_some() {
        return Err(xml_error("duplicate ListObjectVersions field"));
    }
    *slot = Some(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_incomplete_or_ambiguous_version_listings() {
        let cases = [
            "",
            "<ListVersionsResult/>",
            "<Error><Code>InternalError</Code></Error>",
            "<ListVersionsResult><IsTruncated>false</IsTruncated><Version><Key>blob.png</Key><VersionId>v1</VersionId><Size>1</Size></Version>",
            "<ListVersionsResult><IsTruncated>false</IsTruncated><Version><Key>blob.png</Key>",
            "<ListVersionsResult></ListVersionsResult>",
            "<ListVersionsResult><IsTruncated/></ListVersionsResult>",
            "<ListVersionsResult><IsTruncated>invalid</IsTruncated></ListVersionsResult>",
            "<ListVersionsResult><IsTruncated>true</IsTruncated><IsTruncated>false</IsTruncated></ListVersionsResult>",
            "<ListVersionsResult><IsTruncated>false</IsTruncated></ListVersionsResult><Error/>",
            "<ListVersionsResult><IsTruncated>false</IsTruncated></WrongRoot>",
            "<ListVersionsResult><IsTruncated>false</IsTruncated><Version/></ListVersionsResult>",
            "<ListVersionsResult><IsTruncated>false</IsTruncated><ListVersionsResult/></ListVersionsResult>",
            "<ListVersionsResult><IsTruncated>false</IsTruncated><NextKeyMarker>a</NextKeyMarker><NextKeyMarker>b</NextKeyMarker></ListVersionsResult>",
            "<ListVersionsResult><IsTruncated>false</IsTruncated><Version><Key>a</Key><Key>b</Key><VersionId>v1</VersionId></Version></ListVersionsResult>",
        ];
        for xml in cases {
            assert!(
                parse_object_versions_page(xml.as_bytes()).is_err(),
                "accepted {xml}"
            );
        }
    }

    #[test]
    fn accepts_complete_empty_page_with_explicit_pagination_state() {
        let page = parse_object_versions_page(b"<?xml version=\"1.0\"?><s3:ListVersionsResult xmlns:s3=\"http://s3.amazonaws.com/doc/2006-03-01/\"><s3:IsTruncated>false</s3:IsTruncated></s3:ListVersionsResult>").unwrap();
        assert!(page.entries.is_empty());
        assert!(!page.is_truncated);
    }

    #[test]
    fn parse_object_versions_page_includes_objects_delete_markers_and_dual_markers() {
        let page = parse_object_versions_page(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<ListVersionsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>buzz-media</Name>
  <Prefix>_meta/tenant/</Prefix>
  <KeyMarker>_meta/tenant/a.json</KeyMarker>
  <VersionIdMarker>v-old</VersionIdMarker>
  <MaxKeys>2</MaxKeys>
  <IsTruncated>true</IsTruncated>
  <NextKeyMarker>_meta/tenant/a.json</NextKeyMarker>
  <NextVersionIdMarker>v-new</NextVersionIdMarker>
  <DeleteMarker>
    <Key>_meta/tenant/a.json</Key>
    <VersionId>v-delete</VersionId>
    <IsLatest>true</IsLatest>
  </DeleteMarker>
  <Version>
    <Key>_meta/tenant/a.json</Key>
    <VersionId>v-new</VersionId>
    <IsLatest>false</IsLatest>
    <Size>42</Size>
  </Version>
</ListVersionsResult>"#,
        )
        .expect("parse versions page");

        assert!(page.is_truncated);
        assert_eq!(page.next_key_marker.as_deref(), Some("_meta/tenant/a.json"));
        assert_eq!(page.next_version_id_marker.as_deref(), Some("v-new"));
        assert_eq!(
            page.entries,
            vec![
                ObjectVersionEntry {
                    key: "_meta/tenant/a.json".to_string(),
                    version_id: "v-delete".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "_meta/tenant/a.json".to_string(),
                    version_id: "v-new".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 42,
                },
            ]
        );
    }

    #[test]
    fn parse_object_versions_page_preserves_repeated_interleaved_aws_ordering() {
        let page = parse_object_versions_page(
            br#"<ListVersionsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <IsTruncated>false</IsTruncated>
  <Version><Key>k-a</Key><VersionId>v3</VersionId><Size>3</Size></Version>
  <DeleteMarker><Key>k-a</Key><VersionId>v2</VersionId></DeleteMarker>
  <Version><Key>k-a</Key><VersionId>v1</VersionId><Size>1</Size></Version>
  <DeleteMarker><Key>k-b</Key><VersionId>m2</VersionId></DeleteMarker>
  <Version><Key>k-b</Key><VersionId>m1</VersionId><Size>10</Size></Version>
</ListVersionsResult>"#,
        )
        .expect("parse interleaved versions page");

        assert_eq!(
            page.entries,
            vec![
                ObjectVersionEntry {
                    key: "k-a".to_string(),
                    version_id: "v3".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 3,
                },
                ObjectVersionEntry {
                    key: "k-a".to_string(),
                    version_id: "v2".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "k-a".to_string(),
                    version_id: "v1".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 1,
                },
                ObjectVersionEntry {
                    key: "k-b".to_string(),
                    version_id: "m2".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "k-b".to_string(),
                    version_id: "m1".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 10,
                },
            ]
        );
    }

    #[test]
    fn parse_object_versions_page_handles_marker_only_key_before_versioned_key() {
        let page = parse_object_versions_page(
            br#"<ListVersionsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <IsTruncated>false</IsTruncated>
  <DeleteMarker><Key>k-marker-only</Key><VersionId>d-only</VersionId></DeleteMarker>
  <Version><Key>k-versioned</Key><VersionId>v2</VersionId><Size>20</Size></Version>
  <DeleteMarker><Key>k-versioned</Key><VersionId>d1</VersionId></DeleteMarker>
  <Version><Key>k-versioned</Key><VersionId>v1</VersionId><Size>10</Size></Version>
</ListVersionsResult>"#,
        )
        .expect("parse marker-only and versioned keys");

        assert_eq!(
            page.entries,
            vec![
                ObjectVersionEntry {
                    key: "k-marker-only".to_string(),
                    version_id: "d-only".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "k-versioned".to_string(),
                    version_id: "v2".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 20,
                },
                ObjectVersionEntry {
                    key: "k-versioned".to_string(),
                    version_id: "d1".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "k-versioned".to_string(),
                    version_id: "v1".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 10,
                },
            ]
        );
    }
}
