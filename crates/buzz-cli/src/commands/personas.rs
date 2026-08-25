//! `buzz personas` — create, list, get, and delete kind:30175 agent
//! definitions.
//!
//! A persona is the reusable definition (prompt, runtime, model, behavioral
//! defaults) an agent instance is spawned from. Publishing one here makes it
//! available to the owner's Buzz Desktop; it does NOT start an agent. Launching
//! a running agent mints key material and a NIP-OA auth tag, which stays a
//! Desktop-only operation.

use buzz_core::kind::{KIND_PERSONA, KIND_TEAM};
use buzz_sdk::agent_definitions::{
    build_persona_delete, build_persona_event, event_d_tag, normalize_d_tag,
    persona_content_from_event, team_content_from_event, PersonaEventContent,
};
use buzz_sdk::definition_validation::validate_agent_definition_text;

use super::definitions::{
    fetch_head, list_owned, print_event_json, print_write_response, publish_definition,
    read_body_file, read_snapshot, MAX_AGENT_SNAPSHOT_BYTES, MAX_EVENT_CONTENT_LEN,
};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::sdk_err;
use crate::{PersonaCreateArgs, PersonasCmd};

/// Snapshot envelope written by Buzz Desktop's single-agent export.
const AGENT_SNAPSHOT_FORMAT: &str = "buzz-agent-snapshot";

/// Longest inline SVG avatar Buzz Desktop renders off a persona head.
const MAX_INLINE_SVG_LEN: usize = 8 * 1024;

/// Longest inline raster avatar Buzz Desktop renders off a persona head.
const MAX_INLINE_RASTER_LEN: usize = 256 * 1024;

/// Raster MIME types that survive both a Blossom upload and Desktop's inline
/// avatar reader.
const AVATAR_RASTER_MIMES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Longest edge an avatar is downscaled to before publishing. Generous for the
/// sizes Desktop renders, and small enough that a normalized avatar almost
/// always lands back inside the inline bound.
const MAX_AVATAR_DIMENSION: u32 = 512;

/// Largest source image accepted for decoding.
///
/// `image`'s default limits leave the dimension caps unset, and its allocation
/// cap does not reach the PNG decoder's internal buffers, so without an explicit
/// bound a few KB of input declaring 65535x65535 decodes gigabytes.
const MAX_DECODED_DIMENSION: u32 = 8192;

/// Longest avatar URL Buzz Desktop's reader accepts.
const MAX_AVATAR_URL_LEN: usize = 2048;

/// Concurrent-turn ceiling Buzz Desktop enforces when minting an instance
/// (`resolve_mint_behavioral_defaults`), where the floor is 1. Publishing
/// outside that range yields a persona that fails at launch.
const MAX_PARALLELISM: u32 = 32;

/// What to do with an avatar before publishing the persona.
#[derive(Debug)]
enum AvatarPlan {
    /// Carry this value in the event verbatim.
    Publish(String),
    /// Send these bytes to media storage; the returned URL is what publishes.
    Upload(Vec<u8>),
}

/// Decide how `value` reaches the persona event, given the content bytes still
/// available under the relay's cap.
///
/// Buzz Desktop renders a bounded inline avatar directly off a persona head, so
/// a small image is carried as-is rather than paying an upload round trip. Past
/// either that bound or `inline_budget`, a raster is uploaded and the event
/// carries the URL instead.
fn plan_avatar(value: &str, inline_budget: usize) -> Result<AvatarPlan, CliError> {
    let Some(rest) = value.strip_prefix("data:") else {
        // A plain URL is already the by-reference shape; nothing to decide.
        validate_avatar_url(value)?;
        return Ok(AvatarPlan::Publish(value.to_owned()));
    };

    // Emoji avatars are percent-encoded SVG rather than base64. They are
    // self-contained and tiny, but SVG is not an uploadable Blossom type, so an
    // oversized one has no fallback path.
    if rest.starts_with("image/svg+xml,") {
        let limit = MAX_INLINE_SVG_LEN.min(inline_budget);
        if value.len() > limit {
            return Err(CliError::Usage(format!(
                "SVG avatar is too large to publish ({} bytes, max {limit}); \
                 supply a raster image instead",
                value.len()
            )));
        }
        return Ok(AvatarPlan::Publish(value.to_owned()));
    }

    let mime = rest.split(';').next().unwrap_or_default();
    let Some(payload) = rest.strip_prefix(&format!("{mime};base64,")) else {
        return Err(CliError::Usage(format!(
            "unsupported avatar data URL: expected `data:<image mime>;base64,…` or \
             `data:image/svg+xml,…`, got '{}…'",
            value.chars().take(32).collect::<String>()
        )));
    };
    if !AVATAR_RASTER_MIMES.contains(&mime) {
        return Err(CliError::Usage(format!(
            "unsupported avatar type '{mime}' (expected one of {})",
            AVATAR_RASTER_MIMES.join(", ")
        )));
    }

    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
        .map_err(|e| CliError::Usage(format!("avatar data URL is not valid base64: {e}")))?;

    // Normalize every raster, including one that would already fit inline. A
    // persona event is world-readable and durable, so carrying a camera photo
    // verbatim would publish its EXIF — GPS included — to everyone on the relay,
    // and an inline avatar never passes through the media validator that would
    // otherwise catch it. The decode doubles as a validity check: Desktop
    // renders an inline avatar by base64-decoding it, so a payload that does not
    // decode reports success and leaves a persona with a broken image.
    let (normalized, out_mime) = normalize_avatar(bytes, mime)?;
    let reencoded = format!(
        "data:{out_mime};base64,{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &normalized)
    );
    if reencoded.len() <= MAX_INLINE_RASTER_LEN.min(inline_budget) {
        return Ok(AvatarPlan::Publish(reencoded));
    }
    Ok(AvatarPlan::Upload(normalized))
}

/// Reject an avatar URL Buzz Desktop's reader would drop.
///
/// Desktop renders `avatar_url` only when it parses as http(s), stays under
/// 2 KiB, and carries no whitespace or parentheses (`isSafeHttpUrl` in
/// `personaCatalogRelay.ts`). Publishing past that succeeds on the relay and
/// then renders no avatar, with nothing naming the cause.
fn validate_avatar_url(value: &str) -> Result<(), CliError> {
    let reject = |why: String| {
        Err(CliError::Usage(format!(
            "avatar URL {why}; Buzz Desktop renders only an http(s) URL under \
             {MAX_AVATAR_URL_LEN} bytes with no whitespace or parentheses"
        )))
    };
    if value.len() > MAX_AVATAR_URL_LEN {
        return reject(format!("is {} bytes", value.len()));
    }
    if value
        .chars()
        .any(|c| c.is_whitespace() || c == '(' || c == ')')
    {
        return reject("contains whitespace or parentheses".into());
    }
    match url::Url::parse(value) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => Ok(()),
        Ok(url) => reject(format!("uses scheme '{}'", url.scheme())),
        Err(e) => reject(format!("does not parse: {e}")),
    }
}

/// True when the payload holds more than one frame.
///
/// Walks the container rather than scanning for a magic string, so pixel data
/// that happens to spell `acTL` cannot read as animated — a false positive would
/// skip normalization and put the metadata rejection back.
fn is_animated(bytes: &[u8], mime: &str) -> bool {
    match mime {
        // APNG marks itself with an acTL chunk, which must precede IDAT.
        "image/png" => {
            // Walk chunks only once the payload is actually a PNG. Desktop's
            // exporter labels unknown magic bytes `image/png`, and reading
            // arbitrary bytes as a chunk chain can land on `acTL` by accident —
            // which would skip normalization for a non-PNG payload.
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                return false;
            }
            let mut i = 8;
            while i + 8 <= bytes.len() {
                let len = u32::from_be_bytes(match bytes[i..i + 4].try_into() {
                    Ok(v) => v,
                    Err(_) => return false,
                }) as usize;
                match &bytes[i + 4..i + 8] {
                    b"acTL" => return true,
                    b"IDAT" | b"IEND" => return false,
                    _ => {}
                }
                i = match i.checked_add(12).and_then(|v| v.checked_add(len)) {
                    Some(v) => v,
                    None => return false,
                };
            }
            false
        }
        // An animated WebP is a RIFF file carrying an ANIM chunk.
        "image/webp" => {
            if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
                return false;
            }
            let mut i = 12;
            while i + 8 <= bytes.len() {
                let len = u32::from_le_bytes(match bytes[i + 4..i + 8].try_into() {
                    Ok(v) => v,
                    Err(_) => return false,
                }) as usize;
                if &bytes[i..i + 4] == b"ANIM" {
                    return true;
                }
                // RIFF chunks pad to an even length.
                i = match len
                    .checked_add(len & 1)
                    .and_then(|padded| padded.checked_add(8))
                    .and_then(|advance| i.checked_add(advance))
                {
                    Some(v) => v,
                    None => return false,
                };
            }
            false
        }
        _ => false,
    }
}

/// Downscale and re-encode an avatar, returning the bytes and their new MIME.
///
/// Runs on every raster, whether it is bound for an upload or for the event
/// itself: metadata — EXIF, colour profiles, comments — is an identity channel,
/// which media storage rejects and an inline avatar would carry straight into a
/// world-readable event. Decoding and re-encoding drops all of it by
/// construction, so this cannot drift from the relay's chunk allowlist the way a
/// structural stripper would. EXIF orientation is baked into the pixels first;
/// dropping the tag without applying it would silently rotate the avatar.
///
/// GIF and animated images are the exception, returned whole because re-encoding
/// would flatten them to one frame — so they alone can still carry metadata.
/// WebP re-encodes to PNG: `image`'s WebP encoder is lossless-only and would
/// inflate a lossy source.
fn normalize_avatar(bytes: Vec<u8>, mime: &str) -> Result<(Vec<u8>, &'static str), CliError> {
    // Decoding an animated image yields its first frame only, so re-encoding
    // would silently drop the animation. Leave those alone and let the upload
    // report the metadata error instead of publishing a stilled avatar.
    if is_animated(&bytes, mime) {
        let still_mime = match mime {
            "image/png" => "image/png",
            "image/webp" => "image/webp",
            _ => "image/gif",
        };
        return Ok((bytes, still_mime));
    }

    let (format, out_format, out_mime) = match mime {
        "image/jpeg" => (
            image::ImageFormat::Jpeg,
            image::ImageFormat::Jpeg,
            "image/jpeg",
        ),
        "image/png" => (
            image::ImageFormat::Png,
            image::ImageFormat::Png,
            "image/png",
        ),
        "image/webp" => (
            image::ImageFormat::WebP,
            image::ImageFormat::Png,
            "image/png",
        ),
        // GIF is never re-encoded, so nothing downstream ever decodes it. Check
        // the signature here or an `image/gif` data URL of arbitrary bytes
        // publishes verbatim and renders as nothing.
        _ => {
            return if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
                Ok((bytes, "image/gif"))
            } else {
                Err(CliError::Usage(
                    "cannot process avatar image: not a GIF".into(),
                ))
            }
        }
    };

    let bad = |what: &str| CliError::Usage(format!("cannot process avatar image: {what}"));

    use image::ImageDecoder;
    let reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
    let mut decoder = reader.into_decoder().map_err(|_| bad("undecodable"))?;
    // Checked against the header before any pixel is decoded, so a small file
    // declaring enormous dimensions is refused rather than expanded. The default
    // limits do not do this: they leave both dimension caps unset.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODED_DIMENSION);
    limits.max_image_height = Some(MAX_DECODED_DIMENSION);
    decoder.set_limits(limits).map_err(|_| {
        CliError::Usage(format!(
            "cannot process avatar image: larger than {MAX_DECODED_DIMENSION}px on a side"
        ))
    })?;
    let orientation = decoder
        .orientation()
        .map_err(|_| bad("unreadable orientation"))?;
    let mut image =
        image::DynamicImage::from_decoder(decoder).map_err(|_| bad("undecodable pixels"))?;
    image.apply_orientation(orientation);

    if image.width() > MAX_AVATAR_DIMENSION || image.height() > MAX_AVATAR_DIMENSION {
        image = image.resize(
            MAX_AVATAR_DIMENSION,
            MAX_AVATAR_DIMENSION,
            image::imageops::FilterType::Lanczos3,
        );
    }
    // JPEG cannot carry alpha; encoding RGBA to it fails outright.
    if out_format == image::ImageFormat::Jpeg {
        image = image::DynamicImage::ImageRgb8(image.to_rgb8());
    }

    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, out_format)
        .map_err(|_| bad("could not be re-encoded"))?;
    Ok((out.into_inner(), out_mime))
}

/// Name the fix for an upload the media validator refuses.
///
/// Media storage rejects images carrying metadata — EXIF, colour profiles,
/// comments — because they are an identity channel. Re-encoding drops that from
/// a still image, but an animated one passes through whole (flattening it to a
/// single frame would be worse), so an animated avatar can still draw the relay
/// 422, and the bare status says nothing about what to do. Anything else passes
/// through.
fn explain_upload(err: CliError) -> CliError {
    // Gated on the validator's own 422 and its structured `error` field, not on
    // the rendered message: a 500 that merely mentions a metadata service would
    // otherwise be relabelled a user error the user cannot act on.
    let is_metadata_rejection = match &err {
        CliError::Relay { status: 422, body } => serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(|v| v.as_str())
            .is_some_and(|e| e.contains("metadata")),
        _ => false,
    };
    if !is_metadata_rejection {
        return err;
    }
    CliError::Usage(format!(
        "{err}\n\
         The avatar carries metadata (EXIF, a colour profile, or comments), which \
         media storage refuses. Animated images are the one kind published as-is — \
         re-export it without metadata, supply a still image, or pass --avatar-url \
         with an already-hosted image."
    ))
}

/// Replace `content.avatar_url` with a published URL when the value cannot ride
/// along inside the event.
///
/// The budget is whatever the relay's content cap leaves after the rest of the
/// definition, so a long system prompt pushes a borderline avatar to an upload
/// rather than to a rejected write.
async fn finalize_avatar(
    client: &BuzzClient,
    content: &mut PersonaEventContent,
) -> Result<(), CliError> {
    let Some(value) = content.avatar_url.clone() else {
        return Ok(());
    };
    let without_avatar = {
        let mut probe = content.clone();
        probe.avatar_url = None;
        serde_json::to_string(&probe).map(|s| s.len()).unwrap_or(0)
    };
    let budget = MAX_EVENT_CONTENT_LEN.saturating_sub(without_avatar + 64);

    match plan_avatar(&value, budget)? {
        AvatarPlan::Publish(v) => content.avatar_url = Some(v),
        AvatarPlan::Upload(bytes) => {
            eprintln!(
                "note: avatar is {} bytes, past what a persona event carries inline — uploading",
                bytes.len()
            );
            let blob = client.upload_bytes(bytes).await.map_err(explain_upload)?;
            content.avatar_url = Some(blob.url);
        }
    }
    Ok(())
}

/// Read a local image into the data URL form the avatar planner consumes.
fn avatar_file_to_data_url(path: &str) -> Result<String, CliError> {
    let bytes = std::fs::read(path)
        .map_err(|e| CliError::Usage(format!("cannot read avatar file '{path}': {e}")))?;
    let mime = infer::get(&bytes)
        .map(|t| t.mime_type().to_string())
        .ok_or_else(|| CliError::Usage(format!("cannot determine image type of '{path}'")))?;
    if !AVATAR_RASTER_MIMES.contains(&mime.as_str()) {
        return Err(CliError::Usage(format!(
            "'{path}' is {mime}; avatars must be one of {}",
            AVATAR_RASTER_MIMES.join(", ")
        )));
    }
    Ok(format!(
        "data:{mime};base64,{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
    ))
}

/// Lowercase, dedupe, and check a respond-to allowlist.
///
/// Mirrors `validate_respond_to_allowlist` in Desktop's managed-agent types: an
/// entry that is not 64 hex chars fails at mint, so publishing one yields a
/// persona no agent can be created from.
fn normalize_allowlist(entries: &[String]) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let trimmed = entry.trim();
        if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "has an invalid respondToAllowlist pubkey '{trimmed}' (must be 64 hex chars)"
            ));
        }
        let lower = trimmed.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            out.push(lower);
        }
    }
    Ok(out)
}

/// The v1 `definition` fields a persona event publishes.
///
/// Typed rather than probed field-by-field off a `Value` so a wrong-typed entry
/// is an error instead of a silently dropped one: projecting loosely repairs a
/// malformed export under the user, publishing a persona with a shorter
/// allowlist or name pool than the file described. Unknown fields are ignored
/// so a newer Desktop export still imports, and the runtime-only fields Desktop
/// carries (`sourceIsBuiltin`, the timeouts) are deliberately not projected.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotDefinition {
    name: String,
    system_prompt: Option<String>,
    runtime: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    parallelism: Option<u32>,
    respond_to: Option<String>,
    respond_to_allowlist: Option<Vec<String>>,
    name_pool: Option<Vec<String>>,
}

pub async fn dispatch(cmd: PersonasCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        PersonasCmd::Create(args) => cmd_create(client, args).await,
        PersonasCmd::List { json } => cmd_list(client, json).await,
        PersonasCmd::Get { slug, json } => cmd_get(client, &slug, json).await,
        PersonasCmd::Delete { slug } => cmd_delete(client, &slug).await,
    }
}

/// Resolve `--from` and the individual flags into a slug plus content body.
///
/// Flags win over the file so a snapshot can be published under a different
/// name or backbone without editing it.
fn resolve_create(args: &PersonaCreateArgs) -> Result<(String, PersonaEventContent), CliError> {
    let mut content = PersonaEventContent {
        display_name: String::new(),
        system_prompt: None,
        avatar_url: None,
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
    };
    let mut slug_from_file = None;

    if let Some(path) = &args.from {
        let snapshot = read_snapshot(path, AGENT_SNAPSHOT_FORMAT, MAX_AGENT_SNAPSHOT_BYTES)?;
        let raw_def = snapshot.get("definition").ok_or_else(|| {
            CliError::Usage(format!("'{path}' has no `definition` object to publish"))
        })?;
        let def: SnapshotDefinition = serde_json::from_value(raw_def.clone()).map_err(|e| {
            CliError::Usage(format!(
                "'{path}' definition is not a valid v1 agent snapshot: {e}"
            ))
        })?;

        slug_from_file = Some(def.name.clone());
        content.display_name = def.name.clone();
        content.system_prompt = def.system_prompt;
        content.runtime = def.runtime;
        content.model = def.model;
        content.provider = def.provider;

        // `allowlist` mode is only usable with a non-empty list: Desktop rejects
        // an empty one at instance mint, so publishing the mode without the
        // pubkeys yields a persona no agent can ever be minted from. Carry the
        // list across, and refuse rather than silently strip it.
        content.respond_to = def.respond_to;
        content.respond_to_allowlist =
            normalize_allowlist(&def.respond_to_allowlist.unwrap_or_default())
                .map_err(|e| CliError::Usage(format!("'{path}' {e}")))?;
        match content.respond_to.as_deref() {
            Some("allowlist") if content.respond_to_allowlist.is_empty() => {
                return Err(CliError::Usage(format!(
                    "'{path}' sets respondTo=allowlist but has no `respondToAllowlist` \
                     pubkeys; Buzz Desktop cannot mint an agent from that persona"
                )));
            }
            Some(mode) if !matches!(mode, "owner-only" | "anyone" | "allowlist") => {
                return Err(CliError::Usage(format!(
                    "'{path}' has unknown respondTo '{mode}' \
                     (expected owner-only, anyone, or allowlist)"
                )));
            }
            _ => {}
        }
        content.parallelism = match def.parallelism {
            Some(v) if !(1..=MAX_PARALLELISM).contains(&v) => {
                return Err(CliError::Usage(format!(
                    "'{path}' has parallelism {v}, which Buzz Desktop rejects at \
                     mint (must be between 1 and {MAX_PARALLELISM})"
                )));
            }
            other => other,
        };
        content.name_pool = def.name_pool.unwrap_or_else(|| vec![def.name]);

        // Desktop exports the avatar either by reference or inlined as a data
        // URL, depending on size. Both are shapes a persona event can carry —
        // `finalize_avatar` uploads the inline form only when it is too big to
        // ride along — so take whichever the export used.
        let profile = snapshot.get("profile");
        content.avatar_url = profile
            .and_then(|p| p.get("avatarUrl"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                profile
                    .and_then(|p| p.get("avatarDataUrl"))
                    .and_then(|v| v.as_str())
            })
            .map(str::to_owned);
    }

    // Explicit flags override anything the file supplied.
    if let Some(v) = &args.display_name {
        content.display_name = v.clone();
    }
    if let Some(v) = &args.runtime {
        content.runtime = Some(v.clone());
    }
    if let Some(v) = &args.model {
        content.model = Some(v.clone());
    }
    if let Some(v) = &args.provider {
        content.provider = Some(v.clone());
    }
    match (&args.avatar, &args.avatar_url) {
        (Some(_), Some(_)) => {
            return Err(CliError::Usage(
                "--avatar and --avatar-url are mutually exclusive".into(),
            ))
        }
        (Some(path), None) => content.avatar_url = Some(avatar_file_to_data_url(path)?),
        (None, Some(v)) => content.avatar_url = Some(v.clone()),
        (None, None) => {}
    }
    if let Some(v) = args.respond_to {
        content.respond_to = Some(v.to_wire());
    }
    if let Some(v) = args.parallelism {
        content.parallelism = Some(v);
    }
    match (&args.prompt, &args.prompt_file) {
        (Some(_), Some(_)) => {
            return Err(CliError::Usage(
                "--prompt and --prompt-file are mutually exclusive".into(),
            ))
        }
        (Some(p), None) => content.system_prompt = Some(p.clone()),
        (None, Some(path)) => content.system_prompt = Some(read_body_file(path, "prompt")?),
        (None, None) => {}
    }

    if content.display_name.trim().is_empty() {
        return Err(CliError::Usage(
            "--display-name is required (or supply --from with a definition name)".into(),
        ));
    }
    // Same gate Buzz Desktop applies before it will mint an agent. Publishing
    // past it produces a definition that lands on the relay and then fails to
    // launch, which reads as a Desktop bug rather than a bad definition.
    validate_agent_definition_text(
        &content.display_name,
        content.system_prompt.as_deref().unwrap_or_default(),
    )
    .map_err(CliError::Usage)?;
    if content.name_pool.is_empty() {
        content.name_pool = vec![content.display_name.clone()];
    }

    let raw_slug = args
        .slug
        .clone()
        .or(slug_from_file)
        .unwrap_or_else(|| content.display_name.clone());
    Ok((normalize_d_tag(&raw_slug), content))
}

/// True when the event carries the catalog-discovery tag other members read.
fn event_is_shared(event: &nostr::Event) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().map(String::as_str) == Some("shared")
            && parts.get(1).map(String::as_str) == Some("true")
    })
}

/// True when two personas carry the same definition.
///
/// Compares every field a Desktop snapshot's `definition` block can set. The
/// avatar is not one of them — it rides in `profile`, and every raster is
/// re-encoded on import, so it would never match byte for byte.
fn definitions_match(a: &PersonaEventContent, b: &PersonaEventContent) -> bool {
    a.display_name == b.display_name
        && a.system_prompt == b.system_prompt
        && a.runtime == b.runtime
        && a.model == b.model
        && a.provider == b.provider
        && a.name_pool == b.name_pool
        && a.respond_to == b.respond_to
        && a.respond_to_allowlist == b.respond_to_allowlist
        && a.parallelism == b.parallelism
}

/// Find the published persona an import is a copy of.
///
/// Returns the coordinate to adopt, plus any same-named persona that is *not* a
/// copy — the caller reports that one rather than adopting it.
async fn find_published_persona(
    client: &BuzzClient,
    content: &PersonaEventContent,
) -> Result<(Option<String>, Option<String>), CliError> {
    let mut identical = None;
    let mut same_name = None;
    for event in list_owned(client, KIND_PERSONA).await? {
        let (Some(d_tag), Ok(published)) =
            (event_d_tag(&event), persona_content_from_event(&event))
        else {
            continue;
        };
        if !published
            .display_name
            .eq_ignore_ascii_case(&content.display_name)
        {
            continue;
        }
        if definitions_match(&published, content) {
            identical = Some(d_tag.to_owned());
            break;
        }
        same_name.get_or_insert_with(|| d_tag.to_owned());
    }
    Ok((identical, same_name))
}

async fn cmd_create(client: &BuzzClient, args: PersonaCreateArgs) -> Result<(), CliError> {
    let (mut slug, mut content) = resolve_create(&args)?;

    // A Desktop snapshot carries no id, so the slug derived from its name would
    // mint a second coordinate for a persona Desktop already publishes under its
    // record UUID, leaving two records of the same name. Adopt the published
    // coordinate — but only when its definition is the one being imported. A
    // shared display name alone can be a different persona, and adopting that
    // coordinate would overwrite it. Only on --from without --slug: a
    // flags-built persona declares a new one at the slug its name derives, and
    // an explicit --slug names a coordinate — the caller gets that coordinate
    // or a conflict there, never a silent redirect (same guard as teams --id).
    if args.from.is_some() && args.slug.is_none() {
        let (identical, same_name) = find_published_persona(client, &content).await?;
        match (identical, same_name) {
            (Some(existing), _) if existing != slug => {
                eprintln!(
                    "note: '{}' is already published as '{existing}'",
                    content.display_name
                );
                slug = existing;
            }
            (None, Some(other)) => eprintln!(
                "note: a different persona named '{}' is published as '{other}'; \
                 publishing this one as '{slug}'",
                content.display_name
            ),
            _ => {}
        }
    }

    let head = fetch_head(client, KIND_PERSONA, &slug).await?;
    if head.is_some() && !args.replace {
        return Err(CliError::Conflict(format!(
            "persona '{slug}' already exists — pass --replace to overwrite it"
        )));
    }

    // After the conflict check: an upload that a rejected write would strand
    // leaves an orphan blob in media storage.
    finalize_avatar(client, &mut content).await?;

    // Carry the published state forward: a Desktop snapshot has no `shared`
    // field and the flag defaults off, so replacing a shared persona to fix a
    // typo would silently drop it out of every other member's catalog.
    let shared = args.shared || head.as_ref().is_some_and(event_is_shared);
    let builder = build_persona_event(&slug, &content, shared).map_err(sdk_err)?;
    let (event, response) = publish_definition(client, builder, head.as_ref()).await?;

    let verb = if head.is_some() {
        "replaced"
    } else {
        "created"
    };
    eprintln!(
        "{verb} persona {slug} ({}) — event {}",
        content.display_name,
        event.id.to_hex()
    );
    print_write_response(response, "slug", &slug);
    Ok(())
}

async fn cmd_list(client: &BuzzClient, json: bool) -> Result<(), CliError> {
    let events = list_owned(client, KIND_PERSONA).await?;
    if json {
        let items: Vec<serde_json::Value> = events
            .iter()
            .map(|e| {
                serde_json::json!({
                    "slug": event_d_tag(e),
                    "event_id": e.id.to_hex(),
                    "created_at": e.created_at.as_secs(),
                    "content": persona_content_from_event(e).ok(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&items)
                .map_err(|e| CliError::Other(format!("failed to render JSON: {e}")))?
        );
        return Ok(());
    }

    if events.is_empty() {
        eprintln!("no personas published by this identity");
        return Ok(());
    }
    for event in &events {
        let slug = event_d_tag(event).unwrap_or("<no d-tag>");
        match persona_content_from_event(event) {
            Ok(c) => println!(
                "{slug:<24} {:<24} {:<12} {:<24} {} chars",
                c.display_name,
                c.runtime.unwrap_or_else(|| "-".into()),
                c.model.unwrap_or_else(|| "-".into()),
                c.system_prompt.map_or(0, |p| p.len())
            ),
            Err(e) => println!("{slug:<24} <unparseable content: {e}>"),
        }
    }
    Ok(())
}

async fn cmd_get(client: &BuzzClient, slug: &str, json: bool) -> Result<(), CliError> {
    let slug = normalize_d_tag(slug);
    let event = fetch_head(client, KIND_PERSONA, &slug)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("no persona '{slug}' for this identity")))?;
    if json {
        return print_event_json(&event);
    }

    let c = persona_content_from_event(&event).map_err(sdk_err)?;
    println!("slug:         {slug}");
    println!("display_name: {}", c.display_name);
    println!("runtime:      {}", c.runtime.unwrap_or_else(|| "-".into()));
    println!("model:        {}", c.model.unwrap_or_else(|| "-".into()));
    println!("provider:     {}", c.provider.unwrap_or_else(|| "-".into()));
    println!(
        "respond_to:   {}",
        c.respond_to.unwrap_or_else(|| "-".into())
    );
    println!(
        "parallelism:  {}",
        c.parallelism.map_or("-".into(), |p| p.to_string())
    );
    println!("shared:       {}", event_is_shared(&event));
    println!("event:        {}", event.id.to_hex());
    if let Some(prompt) = c.system_prompt {
        println!("\n{prompt}");
    }
    Ok(())
}

/// Warn about published teams that list `slug` as a member.
///
/// Deleting a persona does not rewrite the teams referencing it, so those teams
/// keep a member id that resolves to nothing. Advisory only: a listing failure
/// must not block the delete the caller asked for.
async fn warn_dangling_team_refs(client: &BuzzClient, slug: &str) {
    let events = match list_owned(client, KIND_TEAM).await {
        Ok(events) => events,
        Err(e) => {
            eprintln!("warning: could not check teams for references to '{slug}': {e}");
            return;
        }
    };
    let referencing: Vec<&str> = events
        .iter()
        .filter(|event| {
            team_content_from_event(event)
                .ok()
                .and_then(|c| c.persona_ids)
                .is_some_and(|ids| ids.iter().any(|id| id == slug))
        })
        .filter_map(event_d_tag)
        .collect();
    if !referencing.is_empty() {
        eprintln!(
            "warning: team(s) {} still list '{slug}' as a member; rerun `buzz teams create` \
             for each with the remaining members",
            referencing.join(", ")
        );
    }
}

async fn cmd_delete(client: &BuzzClient, slug: &str) -> Result<(), CliError> {
    let slug = normalize_d_tag(slug);
    let head = fetch_head(client, KIND_PERSONA, &slug).await?;
    if head.is_none() {
        return Err(CliError::NotFound(format!(
            "no persona '{slug}' for this identity"
        )));
    }
    warn_dangling_team_refs(client, &slug).await;
    let builder =
        build_persona_delete(&slug, &client.keys().public_key().to_hex()).map_err(sdk_err)?;
    // NIP-09 scopes an `a`-tag delete to versions at or before the tombstone's
    // own created_at, so the tombstone must outrank the head it targets. A
    // persona replaced within this same second carries a bumped created_at, and
    // a tombstone stamped with a bare `now` would be silently ignored.
    let (event, response) = publish_definition(client, builder, head.as_ref()).await?;

    // The relay accepts a tombstone that deletes nothing and only debug-logs the
    // miss, so acceptance is not evidence. Re-read the coordinate to confirm.
    //
    // A stale read is not a race: buzz-db documents that "stale deletions can
    // briefly inflate the result set", so a lagging replica can still return the
    // head we just deleted. That head is always STRICTLY older than its own
    // tombstone, which is stamped past it — so a survivor at or past the
    // tombstone's stamp is a different event, one the delete did not cover.
    if let Some(survivor) = fetch_head(client, KIND_PERSONA, &slug).await? {
        if survivor.created_at >= event.created_at {
            return Err(CliError::Conflict(format!(
                "persona '{slug}' still exists (head at {}); a concurrent write raced the delete",
                survivor.created_at.as_secs()
            )));
        }
    }

    eprintln!("deleted persona {slug} — tombstone {}", event.id.to_hex());
    print_write_response(response, "slug", &slug);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> PersonaCreateArgs {
        PersonaCreateArgs {
            slug: None,
            display_name: None,
            prompt: None,
            prompt_file: None,
            runtime: None,
            model: None,
            provider: None,
            avatar: None,
            avatar_url: None,
            respond_to: None,
            parallelism: None,
            shared: false,
            from: None,
            replace: false,
        }
    }

    fn snapshot_file(body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "buzz-personas-{}-{}",
            super::super::definitions::now_secs(),
            body.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.json");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    #[test]
    fn display_name_is_required() {
        let err = resolve_create(&args()).unwrap_err();
        assert!(
            err.to_string().contains("--display-name is required"),
            "{err}"
        );
    }

    /// The CLI must refuse what Desktop refuses, or it publishes definitions
    /// that only fail later, at launch.
    #[test]
    fn desktop_definition_text_rules_are_enforced() {
        let hidden = resolve_create(&PersonaCreateArgs {
            display_name: Some("Review\u{200B}er".into()),
            ..args()
        })
        .unwrap_err();
        assert!(hidden.to_string().contains("U+200B"), "{hidden}");

        let long = resolve_create(&PersonaCreateArgs {
            display_name: Some("a".repeat(129)),
            ..args()
        })
        .unwrap_err();
        assert!(long.to_string().contains("too long"), "{long}");

        // Rendered emoji still pass — the rule targets invisible characters.
        assert!(resolve_create(&PersonaCreateArgs {
            display_name: Some("Code Reviewer 👩‍💻".into()),
            ..args()
        })
        .is_ok());
    }

    #[test]
    fn slug_defaults_to_the_normalized_display_name() {
        let (slug, content) = resolve_create(&PersonaCreateArgs {
            display_name: Some("Code Reviewer".into()),
            ..args()
        })
        .unwrap();
        assert_eq!(slug, "code-reviewer");
        // An absent name pool falls back to the display name so instances have
        // something to be called.
        assert_eq!(content.name_pool, vec!["Code Reviewer"]);
    }

    #[test]
    fn explicit_slug_is_normalized_to_the_relay_grammar() {
        let (slug, _) = resolve_create(&PersonaCreateArgs {
            slug: Some("_Ops".into()),
            display_name: Some("Ops".into()),
            ..args()
        })
        .unwrap();
        assert_eq!(slug, "a_ops");
    }

    #[test]
    fn prompt_and_prompt_file_conflict() {
        let err = resolve_create(&PersonaCreateArgs {
            display_name: Some("X".into()),
            prompt: Some("a".into()),
            prompt_file: Some("b".into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
    }

    /// `--from` must reproduce a Desktop export, since that is the artifact
    /// users actually have on disk.
    #[test]
    fn from_reads_a_desktop_agent_snapshot() {
        let (dir, path) = snapshot_file(
            r#"{
              "format": "buzz-agent-snapshot",
              "version": 1,
              "definition": {
                "name": "Herring",
                "sourceIsBuiltin": false,
                "systemPrompt": "You own output quality.",
                "runtime": "buzz-agent",
                "model": "databricks-kimi-3",
                "parallelism": 24,
                "namePool": ["Herring"]
              },
              "profile": {"displayName": "Herring"},
              "memory": {"level": "none"}
            }"#,
        );
        let (slug, content) = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap();

        assert_eq!(slug, "herring");
        assert_eq!(content.display_name, "Herring");
        assert_eq!(
            content.system_prompt.as_deref(),
            Some("You own output quality.")
        );
        assert_eq!(content.runtime.as_deref(), Some("buzz-agent"));
        assert_eq!(content.model.as_deref(), Some("databricks-kimi-3"));
        assert_eq!(content.parallelism, Some(24));
        assert_eq!(content.name_pool, vec!["Herring"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A flag must beat the file, so one snapshot can be republished under a
    /// different backbone without editing it.
    #[test]
    fn flags_override_the_snapshot() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":1,
                "definition":{"name":"Herring","model":"databricks-kimi-3","runtime":"buzz-agent"}}"#,
        );
        let (slug, content) = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            slug: Some("skeptic".into()),
            model: Some("claude-opus-5[1m]".into()),
            runtime: Some("claude".into()),
            ..args()
        })
        .unwrap();

        assert_eq!(slug, "skeptic");
        assert_eq!(content.model.as_deref(), Some("claude-opus-5[1m]"));
        assert_eq!(content.runtime.as_deref(), Some("claude"));
        assert_eq!(
            content.display_name, "Herring",
            "file supplies what flags omit"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Desktop inlines an exported avatar as a data URL and renders that same
    /// form off a persona head, so carry it rather than dropping the image.
    #[test]
    fn inlined_avatar_is_carried_from_the_snapshot() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":1,
                "definition":{"name":"Herring"},
                "profile":{"displayName":"Herring","avatarDataUrl":"data:image/png;base64,AAAA"}}"#,
        );
        let (_, content) = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap();
        assert_eq!(
            content.avatar_url.as_deref(),
            Some("data:image/png;base64,AAAA")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An explicit `--avatar-url` is the caller's stated intent and must win
    /// over whatever the snapshot happened to inline.
    #[test]
    fn avatar_url_flag_overrides_an_inlined_snapshot_avatar() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":1,
                "definition":{"name":"Herring"},
                "profile":{"avatarDataUrl":"data:image/png;base64,AAAA"}}"#,
        );
        let (_, content) = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            avatar_url: Some("https://example.test/h.png".into()),
            ..args()
        })
        .unwrap();
        assert_eq!(
            content.avatar_url.as_deref(),
            Some("https://example.test/h.png")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── avatar planning ──────────────────────────────────────────────────────

    const BUDGET: usize = 190 * 1024;

    fn raster_data_url(payload_len: usize) -> String {
        format!("data:image/png;base64,{}", "A".repeat(payload_len))
    }

    #[test]
    fn a_plain_url_publishes_verbatim() {
        let url = "https://example.test/h.png";
        assert!(matches!(
            plan_avatar(url, BUDGET).unwrap(),
            AvatarPlan::Publish(v) if v == url
        ));
    }

    /// Small enough for Desktop to render straight off the persona head, so an
    /// upload would be a pointless round trip.
    #[test]
    fn a_small_inline_raster_rides_along() {
        let url = data_url("image/png", &png_of(64, "gradient"));
        match plan_avatar(&url, BUDGET).unwrap() {
            AvatarPlan::Publish(v) => assert!(v.starts_with("data:image/png;base64,"), "{v}"),
            AvatarPlan::Upload(_) => panic!("a 64px avatar must ride inline"),
        }
    }

    /// Emoji avatars are percent-encoded SVG, not base64, and are the one
    /// `data:` form Desktop keeps as-is.
    #[test]
    fn an_emoji_svg_avatar_publishes_verbatim() {
        let url = "data:image/svg+xml,%3Csvg%3E%3C/svg%3E";
        assert!(matches!(
            plan_avatar(url, BUDGET).unwrap(),
            AvatarPlan::Publish(v) if v == url
        ));
    }

    /// SVG is not an uploadable Blossom type, so an oversized one has no
    /// fallback — say so instead of publishing something Desktop drops.
    #[test]
    fn an_oversized_svg_avatar_is_refused() {
        let url = format!("data:image/svg+xml,{}", "%20".repeat(MAX_INLINE_SVG_LEN));
        let err = plan_avatar(&url, BUDGET).unwrap_err();
        assert!(err.to_string().contains("SVG avatar is too large"), "{err}");
    }

    /// Desktop would read it, but it does not leave room for the rest of the
    /// content under the relay's cap — upload rather than get rejected. Nothing
    /// normalization can do brings a photo inside a 1 KiB budget.
    #[test]
    fn a_raster_beyond_the_content_budget_is_uploaded() {
        let url = data_url("image/png", &png_of(1400, "photo"));
        assert!(matches!(
            plan_avatar(&url, 1024).unwrap(),
            AvatarPlan::Upload(bytes) if !bytes.is_empty()
        ));
    }

    /// An oversized payload that is not a decodable image is refused here rather
    /// than after a round trip — media storage would reject it anyway.
    #[test]
    fn an_oversized_non_image_payload_is_refused() {
        let url = raster_data_url(MAX_INLINE_RASTER_LEN + 4);
        let err = plan_avatar(&url, usize::MAX).unwrap_err();
        assert!(err.to_string().contains("cannot process avatar"), "{err}");
    }

    /// 1x1 transparent PNG.
    fn png_bytes() -> Vec<u8> {
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
        )
        .unwrap()
    }

    fn temp_file(name: &str, bytes: &[u8]) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "buzz-avatar-{}-{name}",
            super::super::definitions::now_secs()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn avatar_file_becomes_a_publishable_data_url() {
        let (dir, path) = temp_file("h.png", &png_bytes());
        let (_, content) = resolve_create(&PersonaCreateArgs {
            display_name: Some("Herring".into()),
            avatar: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap();
        let avatar = content.avatar_url.expect("avatar set");
        assert!(avatar.starts_with("data:image/png;base64,"), "{avatar}");
        assert!(matches!(
            plan_avatar(&avatar, BUDGET).unwrap(),
            AvatarPlan::Publish(_)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Blossom rejects a non-image, and Desktop would not render one — catch it
    /// locally rather than after a round trip.
    #[test]
    fn a_non_image_avatar_file_is_refused() {
        let (dir, path) = temp_file("notes.txt", b"not an image");
        let err = resolve_create(&PersonaCreateArgs {
            display_name: Some("Herring".into()),
            avatar: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("image type"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn avatar_and_avatar_url_conflict() {
        let err = resolve_create(&PersonaCreateArgs {
            display_name: Some("Herring".into()),
            avatar: Some("h.png".into()),
            avatar_url: Some("https://example.test/h.png".into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn an_unsupported_data_mime_is_refused() {
        let err = plan_avatar("data:application/pdf;base64,AAAA", BUDGET).unwrap_err();
        assert!(err.to_string().contains("unsupported avatar"), "{err}");
    }

    #[test]
    fn malformed_base64_is_refused() {
        let url = format!(
            "data:image/png;base64,{}",
            "!".repeat(MAX_INLINE_RASTER_LEN)
        );
        let err = plan_avatar(&url, BUDGET).unwrap_err();
        assert!(err.to_string().contains("not valid base64"), "{err}");
    }

    /// Builds a PNG of `dim`x`dim`. The pattern decides how well PNG compresses
    /// it, which is what decides inline vs. upload after normalization:
    /// `"gradient"` is flat-art/logo-like, `"photo"` carries photographic
    /// detail, `"noise"` is the incompressible worst case.
    fn png_of(dim: u32, pattern: &str) -> Vec<u8> {
        let mut img = image::RgbaImage::new(dim, dim);
        let span = dim.max(1);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = match pattern {
                "noise" => image::Rgba([
                    (x * 7 + y * 13) as u8,
                    (x * 31 + y * 17) as u8,
                    (x * 3 + y * 91) as u8,
                    255,
                ]),
                "photo" => {
                    let b =
                        (127.0 + 100.0 * (x as f32 / 37.0).sin() * (y as f32 / 41.0).cos()) as u8;
                    image::Rgba([b, b / 4 * 3, 255 - b, 255])
                }
                _ => image::Rgba([(x * 255 / span) as u8, (y * 255 / span) as u8, 128, 255]),
            };
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn data_url(mime: &str, bytes: &[u8]) -> String {
        format!(
            "data:{mime};base64,{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
        )
    }

    /// True when `bytes` carries any PNG chunk media storage treats as a
    /// metadata channel.
    fn png_has_metadata(bytes: &[u8]) -> bool {
        let mut i = 8;
        while i + 12 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
            let kind = &bytes[i + 4..i + 8];
            if matches!(kind, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"iCCP") {
                return true;
            }
            i = i + 12 + len;
        }
        false
    }

    /// An avatar that does not fit the budget left by the rest of the definition
    /// is normalized and re-checked, so flat art publishes inline instead of
    /// paying an upload — and media storage never sees it.
    #[test]
    fn an_avatar_over_budget_is_normalized_back_inside_it() {
        let url = data_url("image/png", &png_of(2000, "gradient"));
        // A budget the source cannot fit, which a downscaled copy comfortably can.
        let budget = url.len() - 1;

        match plan_avatar(&url, budget).unwrap() {
            AvatarPlan::Publish(v) => {
                assert!(v.len() <= budget, "still {} bytes", v.len());
                assert!(v.len() < url.len(), "normalization must shrink it");
                assert!(v.starts_with("data:image/png;base64,"), "{}", &v[..40]);
            }
            AvatarPlan::Upload(b) => {
                panic!("expected inline; normalized to {} raw bytes", b.len())
            }
        }
    }

    /// PNG is lossless, so photographic detail stays past the inline bound even
    /// at 512px and still goes to media storage. What matters is that the bytes
    /// handed to the upload are normalized: that is what the validator refused
    /// before, and it is why the avatar no longer has to be metadata-free
    /// already.
    #[test]
    fn a_photographic_avatar_uploads_normalized_rather_than_inline() {
        let src = png_of(1400, "photo");
        let url = data_url("image/png", &src);
        match plan_avatar(&url, BUDGET).unwrap() {
            AvatarPlan::Upload(bytes) => {
                assert!(bytes.len() < src.len(), "normalization must shrink it");
                assert!(!png_has_metadata(&bytes));
                let (w, h) = imagesize_of(&bytes);
                assert!(
                    w <= MAX_AVATAR_DIMENSION && h <= MAX_AVATAR_DIMENSION,
                    "{w}x{h}"
                );
            }
            AvatarPlan::Publish(v) => panic!("expected an upload; got {} inline bytes", v.len()),
        }
    }

    /// A `dim`x`dim` PNG carrying a `tEXt` chunk, the way a camera or editor
    /// leaves one.
    fn png_with_metadata(dim: u32) -> Vec<u8> {
        let mut src = png_of(dim, "noise");
        let ihdr_end = 8 + 8 + 13 + 4;
        let payload = b"Software\x00Definitely A Real Camera";
        let mut chunk = (payload.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(payload);
        let mut crc = b"tEXt".to_vec();
        crc.extend_from_slice(payload);
        chunk.extend_from_slice(&crc32(&crc).to_be_bytes());
        src.splice(ihdr_end..ihdr_end, chunk);
        assert!(png_has_metadata(&src), "fixture must carry metadata");
        src
    }

    /// Re-encoding drops metadata by construction, which is what makes the
    /// upload fallback survive the media validator.
    #[test]
    fn normalization_drops_png_metadata() {
        let (out, mime) = normalize_avatar(png_with_metadata(1400), "image/png").unwrap();
        assert_eq!(mime, "image/png");
        assert!(!png_has_metadata(&out), "metadata survived normalization");
    }

    /// The subtlest failure this guards: dropping the EXIF tag without applying
    /// it leaves the pixels unrotated, so the avatar publishes sideways. The
    /// rotation must be baked in and the tag gone.
    #[test]
    fn exif_orientation_is_baked_into_the_pixels() {
        use image::ImageEncoder;
        let source = image::RgbImage::from_fn(2, 3, |x, y| {
            image::Rgb([(x * 80) as u8, (y * 60) as u8, 32])
        });
        let mut encoded = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 95)
            .write_image(
                source.as_raw(),
                source.width(),
                source.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();

        // Minimal little-endian Exif IFD with Orientation=6 (rotate 90°).
        let mut exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0".to_vec();
        exif.extend_from_slice(&[
            0x12, 0x01, // Orientation tag
            0x03, 0x00, // SHORT
            0x01, 0x00, 0x00, 0x00, // count=1
            0x06, 0x00, 0x00, 0x00, // value=6
            0x00, 0x00, 0x00, 0x00, // next IFD
        ]);
        let segment_len = (exif.len() + 2) as u16;
        let mut oriented = encoded[..2].to_vec();
        oriented.extend_from_slice(&[0xff, 0xe1]);
        oriented.extend_from_slice(&segment_len.to_be_bytes());
        oriented.extend_from_slice(&exif);
        oriented.extend_from_slice(&encoded[2..]);

        let (out, mime) = normalize_avatar(oriented, "image/jpeg").unwrap();
        assert_eq!(mime, "image/jpeg");
        assert_eq!(imagesize_of(&out), (3, 2), "rotation was not applied");
        assert!(
            !out.windows(6).any(|w| w == b"Exif\0\0"),
            "the Exif segment survived"
        );
    }

    /// Re-encoding an APNG would publish its first frame as a still image.
    #[test]
    fn an_animated_png_is_not_flattened() {
        let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
        apng.extend_from_slice(&13u32.to_be_bytes());
        apng.extend_from_slice(b"IHDR");
        apng.extend_from_slice(&[0; 13 + 4]);
        apng.extend_from_slice(&8u32.to_be_bytes());
        apng.extend_from_slice(b"acTL");
        apng.extend_from_slice(&[0; 8 + 4]);
        assert!(is_animated(&apng, "image/png"), "fixture must read as APNG");

        let (out, mime) = normalize_avatar(apng.clone(), "image/png").unwrap();
        assert_eq!(out, apng, "apng bytes must survive untouched");
        assert_eq!(mime, "image/png");
    }

    /// Pixel data spelling `acTL` must not read as animated — that would skip
    /// normalization and put the metadata rejection back.
    #[test]
    fn an_actl_sequence_in_pixel_data_does_not_read_as_animated() {
        let mut src = png_of(64, "gradient");
        let idat = src
            .windows(4)
            .position(|w| w == b"IDAT")
            .expect("fixture has IDAT");
        src.splice(idat + 8..idat + 12, b"acTL".iter().copied());
        assert!(!is_animated(&src, "image/png"));
    }

    #[test]
    fn an_animated_webp_is_not_flattened() {
        let mut anim = b"RIFF\0\0\0\0WEBP".to_vec();
        anim.extend_from_slice(b"VP8X");
        anim.extend_from_slice(&10u32.to_le_bytes());
        anim.extend_from_slice(&[0; 10]);
        anim.extend_from_slice(b"ANIM");
        anim.extend_from_slice(&6u32.to_le_bytes());
        anim.extend_from_slice(&[0; 6]);
        assert!(
            is_animated(&anim, "image/webp"),
            "fixture must read animated"
        );

        let (out, mime) = normalize_avatar(anim.clone(), "image/webp").unwrap();
        assert_eq!(out, anim, "animated webp must survive untouched");
        assert_eq!(mime, "image/webp");
    }

    /// Re-encoding a GIF would flatten its animation, so it passes through and
    /// keeps the actionable upload error instead.
    #[test]
    fn an_animated_gif_is_never_re_encoded() {
        let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
        let (out, mime) = normalize_avatar(gif.clone(), "image/gif").unwrap();
        assert_eq!(out, gif, "gif bytes must survive untouched");
        assert_eq!(mime, "image/gif");
    }

    /// `image`'s WebP encoder is lossless-only, so re-encoding a lossy WebP can
    /// inflate it. PNG is the predictable container.
    #[test]
    fn webp_normalizes_to_png() {
        let src = png_of(900, "gradient");
        let webp = {
            let img = image::load_from_memory(&src).unwrap();
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::WebP).unwrap();
            out.into_inner()
        };
        let (out, mime) = normalize_avatar(webp, "image/webp").unwrap();
        assert_eq!(mime, "image/png");
        assert!(out.starts_with(b"\x89PNG"), "expected PNG output");
    }

    #[test]
    fn normalization_downscales_to_the_avatar_bound() {
        let (out, _) = normalize_avatar(png_of(1400, "gradient"), "image/png").unwrap();
        let (w, h) = imagesize_of(&out);
        assert!(
            w <= MAX_AVATAR_DIMENSION && h <= MAX_AVATAR_DIMENSION,
            "got {w}x{h}"
        );
        assert!(
            w == MAX_AVATAR_DIMENSION || h == MAX_AVATAR_DIMENSION,
            "got {w}x{h}"
        );
    }

    /// The leak an inline avatar makes easy to miss: a phone photo small enough
    /// to ride along never reaches the media validator, so publishing it
    /// verbatim would put its EXIF — GPS included — in a world-readable event.
    #[test]
    fn an_inline_avatar_is_stripped_of_metadata() {
        let url = data_url("image/png", &png_with_metadata(64));
        match plan_avatar(&url, BUDGET).unwrap() {
            AvatarPlan::Publish(v) => {
                let payload = v.strip_prefix("data:image/png;base64,").expect(&v);
                let bytes =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
                        .unwrap();
                assert!(!png_has_metadata(&bytes), "metadata reached the event");
            }
            AvatarPlan::Upload(_) => panic!("a 64px avatar must ride inline"),
        }
    }

    /// A few KB declaring 65535x65535 would otherwise decode gigabytes: the
    /// crate's default limits leave both dimension caps unset.
    #[test]
    fn a_decompression_bomb_is_refused() {
        let mut ihdr = 65535u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&65535u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
        let mut chunk = b"IHDR".to_vec();
        chunk.extend_from_slice(&ihdr);

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32(&chunk).to_be_bytes());
        // The header is all the check reads, but the decoder is only constructed
        // once it reaches the pixel data.
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IDAT");
        png.extend_from_slice(&crc32(b"IDAT").to_be_bytes());

        let err = normalize_avatar(png, "image/png").unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("larger than {MAX_DECODED_DIMENSION}px")),
            "{err}"
        );
    }

    /// Desktop drops an avatar URL that is not http(s), so publishing one
    /// succeeds on the relay and then renders nothing, with no cause named.
    #[test]
    fn an_avatar_url_desktop_would_drop_is_refused() {
        for value in [
            "javascript:alert(1)",
            "ftp://example.test/h.png",
            "https://example.test/a b.png",
            "https://example.test/h(1).png",
            "not a url",
        ] {
            let err = plan_avatar(value, BUDGET).unwrap_err();
            assert!(err.to_string().contains("avatar URL"), "{value}: {err}");
        }
        let long = format!(
            "https://example.test/{}.png",
            "a".repeat(MAX_AVATAR_URL_LEN)
        );
        let err = plan_avatar(&long, BUDGET).unwrap_err();
        assert!(err.to_string().contains("avatar URL"), "{err}");
    }

    fn imagesize_of(bytes: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(bytes).unwrap();
        (img.width(), img.height())
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A short malformed payload takes the inline branch, which skipped the
    /// decode — create reported success and Desktop got an unrenderable avatar.
    #[test]
    fn malformed_base64_is_refused_even_when_it_would_fit_inline() {
        let err = plan_avatar("data:image/png;base64,not-base64!", BUDGET).unwrap_err();
        assert!(err.to_string().contains("not valid base64"), "{err}");
    }

    /// The error preview truncates at a byte offset; a multibyte character
    /// straddling it used to panic instead of reporting a usage error.
    #[test]
    fn an_unsupported_data_url_with_a_multibyte_char_at_the_cut_reports_usage() {
        // Places 'é' across bytes 31-32, so the 32-byte cut lands mid-character.
        let value = format!("data:image/png{}éllo", "x".repeat(17));
        assert!(
            !value.is_char_boundary(32),
            "test needs a split multibyte char"
        );
        let err = plan_avatar(&value, BUDGET).unwrap_err();
        assert!(err.to_string().contains("unsupported avatar"), "{err}");
    }

    #[test]
    fn a_metadata_rejection_names_the_fix() {
        let raw = CliError::Relay {
            status: 422,
            body: r#"{"error":"media contains metadata or a non-canonical metadata channel"}"#
                .into(),
        };
        let err = explain_upload(raw).to_string();
        assert!(err.contains("re-export it without metadata"), "{err}");
        assert!(err.contains("--avatar-url"), "{err}");
    }

    /// Only the metadata rejection gets rewritten — an unrelated upload failure
    /// must not be relabelled as a user error the user cannot act on.
    #[test]
    fn an_unrelated_upload_failure_passes_through() {
        let raw = CliError::Relay {
            status: 503,
            body: "upstream unavailable".into(),
        };
        let err = explain_upload(raw);
        assert!(matches!(err, CliError::Relay { .. }), "{err}");
        assert!(!err.to_string().contains("re-export"), "{err}");
    }

    /// A failure that merely mentions metadata is not the validator's 422, and
    /// demoting it to a usage error would tell the user to fix their image for
    /// an outage they cannot act on.
    #[test]
    fn an_unrelated_failure_naming_metadata_passes_through() {
        let raw = CliError::Relay {
            status: 500,
            body: r#"{"error":"metadata service unavailable"}"#.into(),
        };
        let err = explain_upload(raw);
        assert!(matches!(err, CliError::Relay { status: 500, .. }), "{err}");
    }

    /// A GIF is never decoded, so without a signature check arbitrary bytes
    /// would publish verbatim under an `image/gif` data URL and render nothing.
    #[test]
    fn a_mislabelled_gif_is_refused() {
        let err = normalize_avatar(b"not a gif at all".to_vec(), "image/gif").unwrap_err();
        assert!(err.to_string().contains("not a GIF"), "{err}");
    }

    /// Desktop's exporter labels unknown magic bytes `image/png`. Walking those
    /// as a chunk chain can hit `acTL` by luck, which would skip normalization
    /// and publish the payload untouched.
    #[test]
    fn non_png_bytes_labelled_png_do_not_read_as_animated() {
        let mut jpeg_ish = b"\xff\xd8\xff\xe0".to_vec();
        jpeg_ish.extend_from_slice(&[0u8; 4]);
        jpeg_ish.extend_from_slice(b"\x00\x00\x00\x00acTL");
        jpeg_ish.extend_from_slice(&[0u8; 16]);
        assert!(!is_animated(&jpeg_ish, "image/png"));
    }

    #[test]
    fn the_shared_tag_is_read_off_a_published_head() {
        let keys = nostr::Keys::generate();
        let (_, content) = resolve_create(&PersonaCreateArgs {
            display_name: Some("Herring".into()),
            ..args()
        })
        .unwrap();
        let sign = |shared| {
            build_persona_event("herring", &content, shared)
                .unwrap()
                .sign_with_keys(&keys)
                .unwrap()
        };
        let (with, without) = (sign(true), sign(false));
        assert!(event_is_shared(&with));
        assert!(!event_is_shared(&without));
    }

    #[test]
    fn from_rejects_a_team_snapshot() {
        let (dir, path) = snapshot_file(r#"{"format":"buzz-team-snapshot","version":1}"#);
        let err = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("expected 'buzz-agent-snapshot'"),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Publishing allowlist mode without pubkeys yields a persona Desktop
    /// refuses to mint an agent from, so it must fail here instead.
    #[test]
    fn allowlist_without_pubkeys_is_refused() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":1,
                "definition":{"name":"Herring","respondTo":"allowlist"}}"#,
        );
        let err = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("respondToAllowlist"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Lowercased and deduped on the way through, matching what Desktop stores.
    #[test]
    fn allowlist_pubkeys_are_carried_across() {
        let (dir, path) = snapshot_file(&format!(
            r#"{{"format":"buzz-agent-snapshot","version":1,
                "definition":{{"name":"Herring","respondTo":"allowlist",
                "respondToAllowlist":["{}","{}","{}"]}}}}"#,
            "AB".repeat(32),
            "cd".repeat(32),
            "ab".repeat(32),
        ));
        let (_, content) = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap();
        assert_eq!(content.respond_to.as_deref(), Some("allowlist"));
        assert_eq!(
            content.respond_to_allowlist,
            vec!["ab".repeat(32), "cd".repeat(32)]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Desktop rejects a pubkey that is not 64 hex chars at mint, so a snapshot
    /// carrying one publishes a persona no agent can be created from.
    #[test]
    fn a_malformed_allowlist_pubkey_is_refused() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":1,
                "definition":{"name":"Herring","respondTo":"allowlist",
                "respondToAllowlist":["aa"]}}"#,
        );
        let err = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("64 hex chars"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Adoption hinges on this: a shared display name alone can be a different
    /// persona, and adopting its coordinate would overwrite it.
    #[test]
    fn definitions_match_compares_every_snapshot_field() {
        let base = || PersonaEventContent {
            display_name: "Herring".into(),
            system_prompt: Some("Ask the annoying question.".into()),
            avatar_url: None,
            runtime: Some("claude".into()),
            model: Some("claude-opus-5".into()),
            provider: None,
            name_pool: vec!["Herring".into()],
            respond_to: Some("owner-only".into()),
            respond_to_allowlist: Vec::new(),
            parallelism: Some(4),
        };
        assert!(definitions_match(&base(), &base()));

        // The avatar is deliberately outside the comparison: it does not live in
        // the snapshot's `definition` block and is re-encoded on the way in.
        let mut avatared = base();
        avatared.avatar_url = Some("https://example.test/h.png".into());
        assert!(definitions_match(&base(), &avatared));

        let with = |field: &'static str, mutate: fn(&mut PersonaEventContent)| {
            let mut c = base();
            mutate(&mut c);
            (field, c)
        };
        let mutations = [
            with("display_name", |c| c.display_name = "Quinby".into()),
            with("system_prompt", |c| c.system_prompt = None),
            with("runtime", |c| c.runtime = Some("codex".into())),
            with("model", |c| c.model = None),
            with("provider", |c| c.provider = Some("x".into())),
            with("name_pool", |c| c.name_pool.push("Quinby".into())),
            with("respond_to", |c| c.respond_to = Some("anyone".into())),
            with("respond_to_allowlist", |c| {
                c.respond_to_allowlist = vec!["ab".repeat(32)]
            }),
            with("parallelism", |c| c.parallelism = Some(8)),
        ];
        for (field, other) in mutations {
            assert!(
                !definitions_match(&base(), &other),
                "{field} must break the match"
            );
        }
    }

    /// Desktop's mint gate is 1..=32, so a snapshot outside it publishes a
    /// persona that fails at launch. 33 is the first value past the ceiling.
    #[test]
    fn an_out_of_range_snapshot_parallelism_is_refused() {
        for value in ["0", "33", "99"] {
            let (dir, path) = snapshot_file(&format!(
                r#"{{"format":"buzz-agent-snapshot","version":1,
                    "definition":{{"name":"Herring","parallelism":{value}}}}}"#
            ));
            let err = resolve_create(&PersonaCreateArgs {
                from: Some(path.to_str().unwrap().into()),
                ..args()
            })
            .unwrap_err();
            assert!(
                err.to_string().contains("must be between 1 and 32"),
                "{value}: {err}"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// Past `u32` the old projection truncated, so 4294967297 published as 1.
    /// The typed decode refuses it instead of repairing it.
    #[test]
    fn an_overflowing_snapshot_parallelism_is_refused() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":1,
                "definition":{"name":"Herring","parallelism":4294967297}}"#,
        );
        let err = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("not a valid v1 agent snapshot"),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A wrong-typed entry used to vanish through `filter_map`, publishing a
    /// shorter allowlist or name pool than the file described.
    #[test]
    fn a_wrong_typed_definition_field_is_refused() {
        for def in [
            r#""name":"Herring","respondToAllowlist":[42]"#,
            r#""name":"Herring","namePool":["Herring",null]"#,
            r#""name":"Herring","systemPrompt":7"#,
            r#""name":42"#,
        ] {
            let (dir, path) = snapshot_file(&format!(
                r#"{{"format":"buzz-agent-snapshot","version":1,"definition":{{{def}}}}}"#
            ));
            let err = resolve_create(&PersonaCreateArgs {
                from: Some(path.to_str().unwrap().into()),
                ..args()
            })
            .unwrap_err();
            assert!(
                err.to_string().contains("not a valid v1 agent snapshot"),
                "{def}: {err}"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// Desktop refuses an import past 5 MiB, so publishing one would create a
    /// persona Desktop could never read back.
    #[test]
    fn an_oversized_snapshot_is_refused() {
        let padding = "x".repeat(MAX_AGENT_SNAPSHOT_BYTES);
        let (dir, path) = snapshot_file(&format!(
            r#"{{"format":"buzz-agent-snapshot","version":1,
                "definition":{{"name":"Herring","systemPrompt":"{padding}"}}}}"#
        ));
        let err = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("snapshot limit"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_respond_to_mode_is_refused() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":1,
                "definition":{"name":"Herring","respondTo":"everyone"}}"#,
        );
        let err = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("unknown respondTo 'everyone'"),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A by-reference avatar URL is the shape a persona event carries; dropping
    /// it silently would publish an avatarless persona.
    #[test]
    fn snapshot_avatar_url_is_published() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":1,
                "definition":{"name":"Herring"},
                "profile":{"avatarUrl":"https://example.test/a.png"}}"#,
        );
        let (_, content) = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap();
        assert_eq!(
            content.avatar_url.as_deref(),
            Some("https://example.test/a.png")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn future_snapshot_version_is_refused() {
        let (dir, path) = snapshot_file(
            r#"{"format":"buzz-agent-snapshot","version":2,
                "definition":{"name":"Herring"}}"#,
        );
        let err = resolve_create(&PersonaCreateArgs {
            from: Some(path.to_str().unwrap().into()),
            ..args()
        })
        .unwrap_err();
        assert!(err.to_string().contains("snapshot version 2"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
