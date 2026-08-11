use sha2::{Digest, Sha256};

/// Stable executable-code identity. Mach-O signing replaces the
/// LC_CODE_SIGNATURE payload and updates its size plus the containing
/// __LINKEDIT segment sizes, so those signer-owned fields are excluded; every
/// other byte remains bound. Other formats use the full file.
pub(crate) fn executable_identity_sha256(bytes: &[u8]) -> Result<String, String> {
    let Some(signature) = macho_signature(bytes)? else {
        return Ok(hex::encode(Sha256::digest(bytes)));
    };
    let signature_end = signature
        .offset
        .checked_add(signature.size)
        .ok_or_else(|| "Mach-O code-signature range overflow".to_string())?;
    if signature_end > bytes.len() {
        return Err("Mach-O code-signature range is invalid".into());
    }

    let mut hasher = Sha256::new();
    let mut cursor = 0;
    for (start, size) in signature.normalized_fields {
        let end = start
            .checked_add(size)
            .filter(|end| *end <= signature.offset)
            .ok_or_else(|| "Mach-O signer-owned field range is invalid".to_string())?;
        if start < cursor {
            return Err("overlapping Mach-O signer-owned fields".into());
        }
        hasher.update(&bytes[cursor..start]);
        hasher.update(vec![0_u8; size]);
        cursor = end;
    }
    hasher.update(&bytes[cursor..signature.offset]);
    hasher.update(&bytes[signature_end..]);
    Ok(hex::encode(hasher.finalize()))
}

struct MachoSignature {
    offset: usize,
    size: usize,
    normalized_fields: Vec<(usize, usize)>,
}

fn macho_signature(bytes: &[u8]) -> Result<Option<MachoSignature>, String> {
    let Some(magic) = bytes.get(..4) else {
        return Ok(None);
    };
    let (little_endian, header_size): (bool, usize) = match magic {
        [0xcf, 0xfa, 0xed, 0xfe] => (true, 32),
        [0xce, 0xfa, 0xed, 0xfe] => (true, 28),
        [0xfe, 0xed, 0xfa, 0xcf] => (false, 32),
        [0xfe, 0xed, 0xfa, 0xce] => (false, 28),
        _ => return Ok(None),
    };
    let read_u32 = |offset: usize| -> Result<u32, String> {
        let bytes: [u8; 4] = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| "truncated Mach-O header".to_string())?
            .try_into()
            .map_err(|_| "invalid Mach-O word".to_string())?;
        Ok(if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    let commands = read_u32(16)? as usize;
    let commands_size = read_u32(20)? as usize;
    let commands_end = header_size
        .checked_add(commands_size)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "invalid Mach-O load-command range".to_string())?;
    let mut offset = header_size;
    let mut signature = None;
    let mut normalized_fields = Vec::new();
    for _ in 0..commands {
        let command = read_u32(offset)?;
        let size = read_u32(offset + 4)? as usize;
        if size < 8
            || offset
                .checked_add(size)
                .is_none_or(|end| end > commands_end)
        {
            return Err("invalid Mach-O load command".into());
        }
        match command {
            0x1d => {
                if size < 16 || signature.is_some() {
                    return Err("invalid LC_CODE_SIGNATURE command".into());
                }
                normalized_fields.push((offset + 8, 8));
                signature = Some((
                    read_u32(offset + 8)? as usize,
                    read_u32(offset + 12)? as usize,
                ));
            }
            0x19 if size >= 72 && bytes.get(offset + 8..offset + 24) == Some(linkedit_name()) => {
                normalized_fields.push((offset + 32, 8));
                normalized_fields.push((offset + 48, 8));
            }
            0x1 if size >= 56 && bytes.get(offset + 8..offset + 24) == Some(linkedit_name()) => {
                normalized_fields.push((offset + 28, 4));
                normalized_fields.push((offset + 36, 4));
            }
            _ => {}
        }
        offset += size;
    }
    let Some((signature_offset, signature_size)) = signature else {
        return Ok(None);
    };
    if signature_offset < commands_end {
        return Err("Mach-O code-signature overlaps load commands".into());
    }
    normalized_fields.sort_unstable();
    Ok(Some(MachoSignature {
        offset: signature_offset,
        size: signature_size,
        normalized_fields,
    }))
}

fn linkedit_name() -> &'static [u8] {
    b"__LINKEDIT\0\0\0\0\0\0"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_macho(signature: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0_u8; 120];
        bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        bytes[16..20].copy_from_slice(&2_u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&88_u32.to_le_bytes());
        bytes[32..36].copy_from_slice(&0x19_u32.to_le_bytes());
        bytes[36..40].copy_from_slice(&72_u32.to_le_bytes());
        bytes[40..56].copy_from_slice(linkedit_name());
        bytes[64..72].copy_from_slice(&(signature.len() as u64).to_le_bytes());
        bytes[80..88].copy_from_slice(&(signature.len() as u64).to_le_bytes());
        bytes[104..108].copy_from_slice(&0x1d_u32.to_le_bytes());
        bytes[108..112].copy_from_slice(&16_u32.to_le_bytes());
        bytes[112..116].copy_from_slice(&120_u32.to_le_bytes());
        bytes[116..120].copy_from_slice(&(signature.len() as u32).to_le_bytes());
        bytes.extend_from_slice(signature);
        bytes
    }

    #[test]
    fn signer_payload_does_not_change_code_identity() {
        assert_eq!(
            executable_identity_sha256(&signed_macho(b"adhoc")).unwrap(),
            executable_identity_sha256(&signed_macho(b"owner signature")).unwrap()
        );
    }

    #[test]
    fn ordinary_code_byte_changes_identity() {
        let first = signed_macho(b"signature");
        let mut changed = first.clone();
        changed[8] = 1;
        assert_ne!(
            executable_identity_sha256(&first).unwrap(),
            executable_identity_sha256(&changed).unwrap()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_resigning_preserves_executable_code_identity() {
        let directory = tempfile::tempdir().expect("temporary signing directory");
        let source = std::env::current_exe().expect("current test executable");
        let candidate = directory.path().join("resigned-test-executable");
        std::fs::copy(&source, &candidate).expect("copy executable for signing");
        let before = executable_identity_sha256(
            &std::fs::read(&candidate).expect("read executable before signing"),
        )
        .expect("identity before signing");
        let status = std::process::Command::new("/usr/bin/codesign")
            .args([
                "--force",
                "--sign",
                "-",
                "--options",
                "runtime",
                "--timestamp=none",
            ])
            .arg(&candidate)
            .status()
            .expect("run ad-hoc code signing");
        assert!(status.success(), "ad-hoc code signing must succeed");
        let after = executable_identity_sha256(
            &std::fs::read(&candidate).expect("read executable after signing"),
        )
        .expect("identity after signing");
        assert_eq!(before, after);
    }
}
