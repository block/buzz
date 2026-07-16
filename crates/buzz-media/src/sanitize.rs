//! Fail-closed media classification, metadata removal, and output verification.
//!
//! Authentication is deliberately outside this module: the caller authenticates
//! the hash of the source bytes, while this module returns a new artifact whose
//! hash becomes the public content-addressed identifier.

use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::{Builder, NamedTempFile};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::{MediaConfig, MediaError};

const MAX_TOOL_OUTPUT: usize = 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 25_000_000;
const MAX_ANIMATION_FRAMES: u64 = 1_000;
const MAX_ANIMATION_PIXELS: u64 = 250_000_000;
const MAX_VIDEO_DURATION_SECS: f64 = 600.0;
const MAX_VIDEO_WIDTH: u32 = 3_840;
const MAX_VIDEO_HEIGHT: u32 = 2_160;
const ICC_CANONICAL_DESCRIPTION: &str = "Sanitized color profile";
const ICC_CANONICAL_COPYRIGHT: &str = "Sanitized by Buzz";
const MAX_ICC_TAGS: usize = 4_096;

const ICC_REMOVED_TAGS: &[[u8; 4]] = &[
    *b"B2D0", *b"B2D1", *b"B2D2", *b"B2D3", *b"D2B0", *b"D2B1", *b"D2B2", *b"D2B3", *b"calt",
    *b"targ", *b"dmnd", *b"dmdd", *b"devs", *b"pseq", *b"psid", *b"scrd", *b"vued", *b"mmod",
    *b"dscm", *b"meta", *b"clrt", *b"clot", *b"cloo", *b"meas", *b"resp", *b"rig0", *b"rig2",
    *b"view", *b"tech", *b"ncl2", *b"ncol", *b"aarg", *b"aabg", *b"aagg", *b"vcgt", *b"ndin",
    *b"vcgp",
];

const ICC_RENDERING_TAGS: [[u8; 4]; 24] = [
    *b"A2B0", *b"A2B1", *b"A2B2", *b"B2A0", *b"B2A1", *b"B2A2", *b"bTRC", *b"bXYZ", *b"bkpt",
    *b"chad", *b"chrm", *b"cicp", *b"clro", *b"gamt", *b"gTRC", *b"gXYZ", *b"kTRC", *b"lumi",
    *b"pre0", *b"pre1", *b"pre2", *b"rTRC", *b"rXYZ", *b"wtpt",
];

static TOOL_VERSIONS: OnceLock<ToolVersions> = OnceLock::new();

/// Sanitizer binary versions captured by the startup capability check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolVersions {
    /// Full first version line reported by ExifTool.
    pub exiftool: String,
    /// Full first version line reported by FFmpeg.
    pub ffmpeg: String,
    /// Full first version line reported by ffprobe.
    pub ffprobe: String,
    /// H.264 encoder selected from the verified FFmpeg capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_encoder: Option<String>,
}

/// Return the startup-verified sanitizer versions for private audit records.
pub fn tool_versions() -> Option<&'static ToolVersions> {
    TOOL_VERSIONS.get()
}

/// High-level class used for route enforcement and bounded metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaClass {
    Image,
    Video,
    Audio,
}

impl MediaClass {
    /// Stable, bounded metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

/// Content-derived media information. Request MIME types and filenames are
/// never used to construct this value.
#[derive(Debug, Clone)]
pub struct MediaProbe {
    pub class: MediaClass,
    pub mime: String,
    pub ext: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_secs: Option<f64>,
    pub frame_count: Option<u64>,
}

/// A verified artifact ready for content-addressed publication.
pub struct SanitizedMedia {
    pub file: NamedTempFile,
    pub probe: MediaProbe,
}

/// Verify required executables at relay startup. This intentionally fails
/// closed: a deployment without its privacy controls must not accept uploads.
pub async fn validate_toolchain(config: &MediaConfig) -> Result<(), MediaError> {
    let exiftool = successful_version(&config.exiftool_path, "-ver").await?;
    let ffmpeg = successful_version(&config.ffmpeg_path, "-version").await?;
    let ffprobe = successful_version(&config.ffprobe_path, "-version").await?;
    reject_nonredistributable_build(&config.ffmpeg_path).await?;
    reject_nonredistributable_build(&config.ffprobe_path).await?;
    let encoders = run_tool(
        &config.ffmpeg_path,
        &["-hide_banner", "-encoders"],
        Duration::from_secs(15),
    )
    .await?;
    let encoders = String::from_utf8_lossy(&encoders.stdout);
    let video_encoder = select_video_encoder(&encoders).ok_or(MediaError::ToolUnavailable)?;
    for required in ["aac"] {
        if !encoders.contains(required) {
            return Err(MediaError::ToolUnavailable);
        }
    }
    let decoders = run_tool(
        &config.ffmpeg_path,
        &["-hide_banner", "-decoders"],
        Duration::from_secs(15),
    )
    .await?;
    let decoders = String::from_utf8_lossy(&decoders.stdout);
    for required in [
        "h264",
        "hevc",
        "vp8",
        "vp9",
        "av1",
        "aac",
        "mp3",
        "flac",
        "vorbis",
        "opus",
        "pcm_s16le",
    ] {
        if !decoders.contains(required) {
            return Err(MediaError::ToolUnavailable);
        }
    }
    let bitstream_filters = run_tool(
        &config.ffmpeg_path,
        &["-hide_banner", "-bsfs"],
        Duration::from_secs(15),
    )
    .await?;
    if !String::from_utf8_lossy(&bitstream_filters.stdout)
        .lines()
        .any(|line| line.trim() == "filter_units")
    {
        return Err(MediaError::ToolUnavailable);
    }
    let _ = TOOL_VERSIONS.set(ToolVersions {
        exiftool,
        ffmpeg,
        ffprobe,
        video_encoder: Some(video_encoder.to_string()),
    });
    Ok(())
}

async fn reject_nonredistributable_build(program: &str) -> Result<(), MediaError> {
    let output = run_tool(
        program,
        &["-hide_banner", "-buildconf"],
        Duration::from_secs(15),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::ToolUnavailable);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if build_is_nonredistributable(&stdout) || build_is_nonredistributable(&stderr) {
        return Err(MediaError::ToolUnavailable);
    }
    Ok(())
}

fn build_is_nonredistributable(configuration: &str) -> bool {
    configuration
        .split_ascii_whitespace()
        .any(|argument| argument == "--enable-nonfree")
}

fn select_video_encoder(encoders: &str) -> Option<&'static str> {
    ["libopenh264", "libx264"].into_iter().find(|encoder| {
        encoders
            .split_ascii_whitespace()
            .any(|word| word == *encoder)
    })
}

fn video_encoder() -> Result<&'static str, MediaError> {
    TOOL_VERSIONS
        .get()
        .and_then(|versions| versions.video_encoder.as_deref())
        .ok_or(MediaError::ToolUnavailable)
}

async fn successful_version(program: &str, arg: &str) -> Result<String, MediaError> {
    let output = run_tool(program, &[arg], Duration::from_secs(15)).await?;
    if !output.status.success() {
        return Err(MediaError::ToolUnavailable);
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or(MediaError::ToolUnavailable)
}

/// Return `None` for a non-media attachment, a supported probe for accepted
/// media, or `UnsupportedMedia` when a parser recognizes media outside the
/// compliance allowlist.
pub async fn probe_media(
    path: &Path,
    sniff: &[u8],
    config: &MediaConfig,
) -> Result<Option<MediaProbe>, MediaError> {
    if let Some((mime, ext, expected_codec)) = iso_bmff_still_image(sniff) {
        let mut probe = match probe_with_ffprobe(path, config, true).await {
            Ok(probe) => {
                if probe.audio_codec.is_some()
                    || probe.video_codec.as_deref() != Some(expected_codec)
                {
                    return Err(MediaError::SanitizationFailed);
                }
                probe
            }
            Err(_) => {
                let (width, height, frame_count) = exiftool_image_dimensions(path, config).await?;
                MediaProbe {
                    class: MediaClass::Image,
                    mime: mime.to_string(),
                    ext: ext.to_string(),
                    video_codec: None,
                    audio_codec: None,
                    width: Some(width),
                    height: Some(height),
                    duration_secs: None,
                    frame_count,
                }
            }
        };
        probe.class = MediaClass::Image;
        probe.mime = mime.to_string();
        probe.ext = ext.to_string();
        validate_media_limits(&probe)?;
        return Ok(Some(probe));
    }
    let mut recognized_media = false;
    if let Some(kind) = infer::get(sniff) {
        if let Some((mime, ext)) = supported_image(kind.mime_type()) {
            let mut probe = match probe_with_ffprobe(path, config, true).await {
                Ok(probe) => probe,
                Err(_) => {
                    let (width, height) = image_dimensions(path)?;
                    MediaProbe {
                        class: MediaClass::Image,
                        mime: mime.to_string(),
                        ext: ext.to_string(),
                        video_codec: None,
                        audio_codec: None,
                        width: Some(width),
                        height: Some(height),
                        duration_secs: None,
                        frame_count: None,
                    }
                }
            };
            probe.class = MediaClass::Image;
            probe.mime = mime.to_string();
            probe.ext = ext.to_string();
            validate_media_limits(&probe)?;
            return Ok(Some(probe));
        }
        if kind.mime_type().starts_with("image/") {
            return Err(MediaError::UnsupportedMedia(kind.mime_type().to_string()));
        }
        recognized_media =
            kind.mime_type().starts_with("video/") || kind.mime_type().starts_with("audio/");
    }

    let probe = match probe_with_ffprobe(path, config, false).await {
        Ok(probe) => probe,
        Err(MediaError::SanitizationFailed) if !recognized_media => return Ok(None),
        Err(error) => return Err(error),
    };
    match probe.class {
        MediaClass::Video => {
            if !matches!(probe.ext.as_str(), "mp4" | "mov" | "webm" | "mkv") {
                return Err(MediaError::UnsupportedMedia(probe.mime));
            }
        }
        MediaClass::Audio => {
            if !matches!(
                probe.ext.as_str(),
                "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus"
            ) {
                return Err(MediaError::UnsupportedMedia(probe.mime));
            }
        }
        MediaClass::Image => {}
    }
    validate_media_limits(&probe)?;
    Ok(Some(probe))
}

fn image_dimensions(path: &Path) -> Result<(u32, u32), MediaError> {
    let dimensions = imagesize::size(path).map_err(|_| MediaError::SanitizationFailed)?;
    let width = u32::try_from(dimensions.width).map_err(|_| MediaError::ImageTooLarge)?;
    let height = u32::try_from(dimensions.height).map_err(|_| MediaError::ImageTooLarge)?;
    if width == 0 || height == 0 {
        return Err(MediaError::SanitizationFailed);
    }
    Ok((width, height))
}

async fn exiftool_image_dimensions(
    path: &Path,
    config: &MediaConfig,
) -> Result<(u32, u32, Option<u64>), MediaError> {
    let args = [
        "-j".to_string(),
        "-n".to_string(),
        "-ImageWidth".to_string(),
        "-ImageHeight".to_string(),
        "-FrameCount".to_string(),
        "-ImageCount".to_string(),
        path_string(path),
    ];
    let output = run_tool(
        &config.exiftool_path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(config.image_process_timeout_secs),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::SanitizationFailed);
    }
    let documents: Vec<Value> =
        serde_json::from_slice(&output.stdout).map_err(|_| MediaError::SanitizationFailed)?;
    let document = documents
        .first()
        .and_then(Value::as_object)
        .ok_or(MediaError::SanitizationFailed)?;
    let parse_u64 = |name: &str| {
        document.get(name).and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
    };
    let width = parse_u64("ImageWidth")
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(MediaError::SanitizationFailed)?;
    let height = parse_u64("ImageHeight")
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(MediaError::SanitizationFailed)?;
    let frame_count = parse_u64("FrameCount").or_else(|| parse_u64("ImageCount"));
    Ok((width, height, frame_count))
}

/// Remove metadata and return a separately verified output artifact.
pub async fn sanitize(
    source: &Path,
    source_probe: &MediaProbe,
    config: &MediaConfig,
) -> Result<SanitizedMedia, MediaError> {
    let (file, expected_class) = match source_probe.class {
        MediaClass::Image => (
            sanitize_image(source, source_probe, config).await?,
            MediaClass::Image,
        ),
        MediaClass::Video => (
            sanitize_video(source, source_probe, config).await?,
            MediaClass::Video,
        ),
        MediaClass::Audio => (
            sanitize_audio(source, source_probe, config).await?,
            MediaClass::Audio,
        ),
    };

    verify_forbidden_metadata(file.path(), expected_class, config).await?;
    verify_embedded_icc(file.path(), expected_class, config).await?;
    let sniff = read_sniff(file.path()).await?;
    let output_probe = probe_media(file.path(), &sniff, config)
        .await?
        .ok_or(MediaError::SanitizationFailed)?;
    if output_probe.class != expected_class {
        return Err(MediaError::SanitizationFailed);
    }
    verify_stream_shape(&output_probe, file.path(), config).await?;
    verify_no_codec_metadata(&output_probe, file.path(), config).await?;
    Ok(SanitizedMedia {
        file,
        probe: output_probe,
    })
}

async fn sanitize_image(
    source: &Path,
    probe: &MediaProbe,
    config: &MediaConfig,
) -> Result<NamedTempFile, MediaError> {
    if probe.ext == "bmp" {
        let output = temp_with_suffix(".png")?;
        let args = vec![
            "-nostdin".to_string(),
            "-v".to_string(),
            "error".to_string(),
            "-y".to_string(),
            "-protocol_whitelist".to_string(),
            "file,pipe".to_string(),
            "-i".to_string(),
            path_string(source),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map_metadata".to_string(),
            "-1".to_string(),
            "-frames:v".to_string(),
            "1".to_string(),
            path_string(output.path()),
        ];
        require_success(
            &config.ffmpeg_path,
            &args,
            Duration::from_secs(config.image_process_timeout_secs),
        )
        .await?;
        return Ok(output);
    }

    if probe.ext == "tiff" {
        let output = temp_with_suffix(".tiff")?;
        let args = vec![
            "-nostdin".to_string(),
            "-v".to_string(),
            "error".to_string(),
            "-y".to_string(),
            "-protocol_whitelist".to_string(),
            "file,pipe".to_string(),
            "-i".to_string(),
            path_string(source),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map_metadata".to_string(),
            "-1".to_string(),
            "-frames:v".to_string(),
            "1".to_string(),
            "-c:v".to_string(),
            "tiff".to_string(),
            path_string(output.path()),
        ];
        require_success(
            &config.ffmpeg_path,
            &args,
            Duration::from_secs(config.image_process_timeout_secs),
        )
        .await?;
        let args = [
            "-overwrite_original".to_string(),
            "-tagsFromFile".to_string(),
            path_string(source),
            "-ICC_Profile:All".to_string(),
            "-ColorSpaceTags".to_string(),
            "-Orientation".to_string(),
            "-Software=".to_string(),
            path_string(output.path()),
        ];
        require_success(
            &config.exiftool_path,
            &args,
            Duration::from_secs(config.image_process_timeout_secs),
        )
        .await?;
        scrub_embedded_icc(output.path(), config).await?;
        return Ok(output);
    }

    let output = temp_with_suffix(&format!(".{}", probe.ext))?;
    tokio::fs::copy(source, output.path())
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    // Delete everything, excluding the ICC block from deletion, then copy back
    // only rendering-critical color-space and orientation tags from the file.
    let args = [
        "-overwrite_original".to_string(),
        "-all=".to_string(),
        "--ICC_Profile:All".to_string(),
        "-tagsFromFile".to_string(),
        "@".to_string(),
        "-ColorSpaceTags".to_string(),
        "-Orientation".to_string(),
        path_string(output.path()),
    ];
    require_success(
        &config.exiftool_path,
        &args,
        Duration::from_secs(config.image_process_timeout_secs),
    )
    .await?;
    scrub_embedded_icc(output.path(), config).await?;
    Ok(output)
}

async fn scrub_embedded_icc(path: &Path, config: &MediaConfig) -> Result<(), MediaError> {
    let path_arg = path_string(path);
    let output = run_tool(
        &config.exiftool_path,
        &["-b", "-ICC_Profile", &path_arg],
        Duration::from_secs(config.image_process_timeout_secs),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::SanitizationFailed);
    }
    if output.stdout.is_empty() {
        return Ok(());
    }

    let scrubbed = scrub_icc_profile(&output.stdout)?;
    let profile = temp_with_suffix(".icc")?;
    tokio::fs::write(profile.path(), scrubbed)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    let assignment = format!("-ICC_Profile<={}", path_string(profile.path()));
    let args = [
        "-overwrite_original".to_string(),
        assignment,
        path_string(path),
    ];
    require_success(
        &config.exiftool_path,
        &args,
        Duration::from_secs(config.image_process_timeout_secs),
    )
    .await
}

fn scrub_icc_profile(profile: &[u8]) -> Result<Vec<u8>, MediaError> {
    const HEADER_SIZE: usize = 128;
    const TAG_TABLE_START: usize = HEADER_SIZE + 4;

    if profile.len() < TAG_TABLE_START || profile.get(36..40) != Some(b"acsp") {
        return Err(MediaError::SanitizationFailed);
    }
    let declared_size = read_icc_u32(profile, 0)?;
    let declared_size =
        usize::try_from(declared_size).map_err(|_| MediaError::SanitizationFailed)?;
    if declared_size < TAG_TABLE_START || declared_size > profile.len() {
        return Err(MediaError::SanitizationFailed);
    }
    let profile = &profile[..declared_size];
    let tag_count = usize::try_from(read_icc_u32(profile, HEADER_SIZE)?)
        .map_err(|_| MediaError::SanitizationFailed)?;
    if tag_count > MAX_ICC_TAGS {
        return Err(MediaError::SanitizationFailed);
    }
    let table_size = tag_count
        .checked_mul(12)
        .and_then(|size| TAG_TABLE_START.checked_add(size))
        .ok_or(MediaError::SanitizationFailed)?;
    if table_size > profile.len() {
        return Err(MediaError::SanitizationFailed);
    }

    let mut tags = Vec::with_capacity(tag_count);
    let mut seen_signatures = HashSet::with_capacity(tag_count);
    let mut retained_input_bytes = 0_usize;
    for index in 0..tag_count {
        let entry = TAG_TABLE_START + index * 12;
        let signature: [u8; 4] = profile[entry..entry + 4]
            .try_into()
            .map_err(|_| MediaError::SanitizationFailed)?;
        if !seen_signatures.insert(signature) {
            return Err(MediaError::SanitizationFailed);
        }
        let offset = usize::try_from(read_icc_u32(profile, entry + 4)?)
            .map_err(|_| MediaError::SanitizationFailed)?;
        let size = usize::try_from(read_icc_u32(profile, entry + 8)?)
            .map_err(|_| MediaError::SanitizationFailed)?;
        let end = offset
            .checked_add(size)
            .ok_or(MediaError::SanitizationFailed)?;
        if offset < table_size || end > profile.len() {
            return Err(MediaError::SanitizationFailed);
        }
        if ICC_REMOVED_TAGS.contains(&signature) {
            continue;
        }
        retained_input_bytes = retained_input_bytes
            .checked_add(size)
            .ok_or(MediaError::SanitizationFailed)?;
        if retained_input_bytes > MAX_TOOL_OUTPUT {
            return Err(MediaError::SanitizationFailed);
        }
        let original = profile
            .get(offset..end)
            .ok_or(MediaError::SanitizationFailed)?;
        let data = match &signature {
            b"desc" => canonical_icc_text(&signature, original, ICC_CANONICAL_DESCRIPTION)?,
            b"cprt" => canonical_icc_text(&signature, original, ICC_CANONICAL_COPYRIGHT)?,
            _ => canonical_icc_rendering_tag(&signature, original)?,
        };
        tags.push((signature, data));
    }
    // Tag-table order has no color-rendering semantics. Canonicalize it so a
    // source profile cannot retain a device fingerprint or covert payload in
    // the permutation of otherwise permitted tags.
    tags.sort_unstable_by_key(|(signature, _)| *signature);

    let new_table_end = TAG_TABLE_START
        .checked_add(
            tags.len()
                .checked_mul(12)
                .ok_or(MediaError::SanitizationFailed)?,
        )
        .ok_or(MediaError::SanitizationFailed)?;
    let mut scrubbed = canonical_icc_header(profile)?.to_vec();
    scrubbed.extend_from_slice(
        &u32::try_from(tags.len())
            .map_err(|_| MediaError::SanitizationFailed)?
            .to_be_bytes(),
    );
    scrubbed.resize(new_table_end, 0);

    for (index, (signature, data)) in tags.into_iter().enumerate() {
        while !scrubbed.len().is_multiple_of(4) {
            scrubbed.push(0);
        }
        let offset = scrubbed.len();
        scrubbed.extend_from_slice(&data);
        let entry = TAG_TABLE_START + index * 12;
        scrubbed[entry..entry + 4].copy_from_slice(&signature);
        scrubbed[entry + 4..entry + 8].copy_from_slice(
            &u32::try_from(offset)
                .map_err(|_| MediaError::SanitizationFailed)?
                .to_be_bytes(),
        );
        scrubbed[entry + 8..entry + 12].copy_from_slice(
            &u32::try_from(data.len())
                .map_err(|_| MediaError::SanitizationFailed)?
                .to_be_bytes(),
        );
    }
    while !scrubbed.len().is_multiple_of(4) {
        scrubbed.push(0);
    }
    let scrubbed_size =
        u32::try_from(scrubbed.len()).map_err(|_| MediaError::SanitizationFailed)?;
    scrubbed[0..4].copy_from_slice(&scrubbed_size.to_be_bytes());
    Ok(scrubbed)
}

fn canonical_icc_header(profile: &[u8]) -> Result<[u8; 128], MediaError> {
    let version_major = *profile.get(8).ok_or(MediaError::SanitizationFailed)?;
    let version_minor_bugfix = *profile.get(9).ok_or(MediaError::SanitizationFailed)?;
    if !matches!(version_major, 2 | 4) {
        return Err(MediaError::SanitizationFailed);
    }
    let profile_class = icc_signature(profile, 12)?;
    if !matches!(
        &profile_class,
        b"scnr" | b"mntr" | b"prtr" | b"link" | b"spac" | b"abst" | b"nmcl"
    ) {
        return Err(MediaError::SanitizationFailed);
    }
    let color_space = icc_signature(profile, 16)?;
    if !valid_icc_color_space(&color_space) {
        return Err(MediaError::SanitizationFailed);
    }
    let connection_space = icc_signature(profile, 20)?;
    if !matches!(&connection_space, b"XYZ " | b"Lab ") {
        return Err(MediaError::SanitizationFailed);
    }
    let rendering_intent = read_icc_u32(profile, 64)?;
    if rendering_intent > 3 {
        return Err(MediaError::SanitizationFailed);
    }

    let mut header = [0_u8; 128];
    header[8] = version_major;
    header[9] = version_minor_bugfix;
    header[12..16].copy_from_slice(&profile_class);
    header[16..20].copy_from_slice(&color_space);
    header[20..24].copy_from_slice(&connection_space);
    header[36..40].copy_from_slice(b"acsp");
    // Profile flags and device attributes are deliberately canonicalized to
    // zero; their reserved bits are an otherwise opaque data channel and they
    // are not part of the color transform.
    header[64..68].copy_from_slice(&rendering_intent.to_be_bytes());
    // ICC.1 v2/v4 mandates the D50 PCS illuminant. Rebuilding the fixed value
    // prevents this header field from becoming an opaque metadata channel.
    header[68..80].copy_from_slice(&[
        0x00, 0x00, 0xf6, 0xd6, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0xd3, 0x2d,
    ]);
    Ok(header)
}

fn icc_signature(profile: &[u8], offset: usize) -> Result<[u8; 4], MediaError> {
    profile
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .ok_or(MediaError::SanitizationFailed)
}

fn valid_icc_color_space(signature: &[u8; 4]) -> bool {
    matches!(
        signature,
        b"XYZ "
            | b"Lab "
            | b"Luv "
            | b"YCbr"
            | b"Yxy "
            | b"RGB "
            | b"GRAY"
            | b"HSV "
            | b"HLS "
            | b"CMYK"
            | b"CMY "
            | b"1CLR"
            | b"2CLR"
            | b"3CLR"
            | b"4CLR"
            | b"5CLR"
            | b"6CLR"
            | b"7CLR"
            | b"8CLR"
            | b"9CLR"
            | b"ACLR"
            | b"BCLR"
            | b"CCLR"
            | b"DCLR"
            | b"ECLR"
            | b"FCLR"
    )
}

fn canonical_icc_text(
    signature: &[u8; 4],
    original: &[u8],
    value: &str,
) -> Result<Vec<u8>, MediaError> {
    if original.get(0..4) == Some(b"mluc") {
        let utf16 = value
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        let mut data = Vec::with_capacity(28 + utf16.len());
        data.extend_from_slice(b"mluc");
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(&1_u32.to_be_bytes());
        data.extend_from_slice(&12_u32.to_be_bytes());
        data.extend_from_slice(b"enUS");
        data.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
        data.extend_from_slice(&28_u32.to_be_bytes());
        data.extend_from_slice(&utf16);
        Ok(data)
    } else if signature == b"desc" && original.get(0..4) == Some(b"desc") {
        let mut data = Vec::with_capacity(91 + value.len());
        data.extend_from_slice(b"desc");
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(&((value.len() + 1) as u32).to_be_bytes());
        data.extend_from_slice(value.as_bytes());
        data.push(0);
        data.extend_from_slice(&[0; 8]);
        data.extend_from_slice(&[0; 2]);
        data.push(0);
        data.extend_from_slice(&[0; 67]);
        Ok(data)
    } else if signature == b"cprt" && original.get(0..4) == Some(b"text") {
        let mut data = Vec::with_capacity(9 + value.len());
        data.extend_from_slice(b"text");
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(value.as_bytes());
        data.push(0);
        Ok(data)
    } else {
        Err(MediaError::SanitizationFailed)
    }
}

fn canonical_icc_rendering_tag(signature: &[u8; 4], data: &[u8]) -> Result<Vec<u8>, MediaError> {
    if !ICC_RENDERING_TAGS.contains(signature) || data.get(4..8) != Some(&[0; 4]) {
        return Err(MediaError::SanitizationFailed);
    }
    let tag_type = data.get(0..4).ok_or(MediaError::SanitizationFailed)?;
    let expected_size = match signature {
        b"A2B0" | b"A2B1" | b"A2B2" | b"B2A0" | b"B2A1" | b"B2A2" | b"gamt" | b"pre0" | b"pre1"
        | b"pre2" => match tag_type {
            b"mft1" => icc_lut8_size(data)?,
            b"mft2" => icc_lut16_size(data)?,
            // mAB/mBA and multi-process elements are standards-compliant but
            // contain nested offset graphs. Reject them until every element
            // can be rebuilt without opaque trailing bytes.
            _ => return Err(MediaError::SanitizationFailed),
        },
        b"bTRC" | b"gTRC" | b"kTRC" | b"rTRC" => icc_curve_size(data)?,
        b"bXYZ" | b"bkpt" | b"gXYZ" | b"lumi" | b"rXYZ" | b"wtpt" if tag_type == b"XYZ " => 20,
        b"chad" if tag_type == b"sf32" => 44,
        b"chrm" if tag_type == b"chrm" => {
            let channels = usize::from(read_icc_u16(data, 8)?);
            12_usize
                .checked_add(
                    channels
                        .checked_mul(8)
                        .ok_or(MediaError::SanitizationFailed)?,
                )
                .ok_or(MediaError::SanitizationFailed)?
        }
        b"cicp" if tag_type == b"cicp" => 12,
        b"clro" if tag_type == b"clro" => {
            let channels = usize::try_from(read_icc_u32(data, 8)?)
                .map_err(|_| MediaError::SanitizationFailed)?;
            12_usize
                .checked_add(channels)
                .ok_or(MediaError::SanitizationFailed)?
        }
        _ => return Err(MediaError::SanitizationFailed),
    };
    let padded_size = expected_size
        .checked_add(3)
        .map(|size| size / 4 * 4)
        .ok_or(MediaError::SanitizationFailed)?;
    if !(expected_size..=padded_size).contains(&data.len())
        || data[expected_size..].iter().any(|byte| *byte != 0)
    {
        return Err(MediaError::SanitizationFailed);
    }
    // ICC writers may include up to the next four-byte boundary in the tag's
    // declared size. Accept only zero alignment bytes and strip them so the
    // rebuilt profile is deterministic and cannot retain a trailing payload.
    Ok(data[..expected_size].to_vec())
}

fn icc_curve_size(data: &[u8]) -> Result<usize, MediaError> {
    match data.get(0..4) {
        Some(b"curv") => {
            let entries = usize::try_from(read_icc_u32(data, 8)?)
                .map_err(|_| MediaError::SanitizationFailed)?;
            12_usize
                .checked_add(
                    entries
                        .checked_mul(2)
                        .ok_or(MediaError::SanitizationFailed)?,
                )
                .ok_or(MediaError::SanitizationFailed)
        }
        Some(b"para") => {
            if data.get(10..12) != Some(&[0; 2]) {
                return Err(MediaError::SanitizationFailed);
            }
            let parameters = match read_icc_u16(data, 8)? {
                0 => 1,
                1 => 3,
                2 => 4,
                3 => 5,
                4 => 7,
                _ => return Err(MediaError::SanitizationFailed),
            };
            12_usize
                .checked_add(parameters * 4)
                .ok_or(MediaError::SanitizationFailed)
        }
        _ => Err(MediaError::SanitizationFailed),
    }
}

fn icc_lut8_size(data: &[u8]) -> Result<usize, MediaError> {
    if data.len() < 48 || data.get(11) != Some(&0) {
        return Err(MediaError::SanitizationFailed);
    }
    let inputs = usize::from(data[8]);
    let outputs = usize::from(data[9]);
    let grid = usize::from(data[10]);
    if inputs == 0 || outputs == 0 || grid < 2 {
        return Err(MediaError::SanitizationFailed);
    }
    let clut_points = checked_icc_power(grid, inputs)?;
    48_usize
        .checked_add(
            inputs
                .checked_mul(256)
                .ok_or(MediaError::SanitizationFailed)?,
        )
        .and_then(|size| size.checked_add(clut_points.checked_mul(outputs)?))
        .and_then(|size| size.checked_add(outputs.checked_mul(256)?))
        .ok_or(MediaError::SanitizationFailed)
}

fn icc_lut16_size(data: &[u8]) -> Result<usize, MediaError> {
    if data.len() < 52 || data.get(11) != Some(&0) {
        return Err(MediaError::SanitizationFailed);
    }
    let inputs = usize::from(data[8]);
    let outputs = usize::from(data[9]);
    let grid = usize::from(data[10]);
    let input_entries = usize::from(read_icc_u16(data, 48)?);
    let output_entries = usize::from(read_icc_u16(data, 50)?);
    if inputs == 0 || outputs == 0 || grid < 2 || input_entries < 2 || output_entries < 2 {
        return Err(MediaError::SanitizationFailed);
    }
    let clut_points = checked_icc_power(grid, inputs)?;
    let input_bytes = inputs
        .checked_mul(input_entries)
        .and_then(|size| size.checked_mul(2))
        .ok_or(MediaError::SanitizationFailed)?;
    52_usize
        .checked_add(input_bytes)
        .and_then(|size| size.checked_add(clut_points.checked_mul(outputs)?.checked_mul(2)?))
        .and_then(|size| size.checked_add(outputs.checked_mul(output_entries)?.checked_mul(2)?))
        .ok_or(MediaError::SanitizationFailed)
}

fn checked_icc_power(base: usize, exponent: usize) -> Result<usize, MediaError> {
    (0..exponent).try_fold(1_usize, |value, _| {
        value
            .checked_mul(base)
            .ok_or(MediaError::SanitizationFailed)
    })
}

fn read_icc_u32(bytes: &[u8], offset: usize) -> Result<u32, MediaError> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or(MediaError::SanitizationFailed)
}

fn read_icc_u16(bytes: &[u8], offset: usize) -> Result<u16, MediaError> {
    bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_be_bytes)
        .ok_or(MediaError::SanitizationFailed)
}

async fn verify_embedded_icc(
    path: &Path,
    class: MediaClass,
    config: &MediaConfig,
) -> Result<(), MediaError> {
    let path_arg = path_string(path);
    let timeout_secs = icc_verification_timeout_secs(
        class,
        config.image_process_timeout_secs,
        config.av_process_timeout_secs,
    );
    let output = run_tool(
        &config.exiftool_path,
        &["-b", "-ICC_Profile", &path_arg],
        Duration::from_secs(timeout_secs),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::SanitizationFailed);
    }
    if output.stdout.is_empty() {
        return Ok(());
    }
    let expected = scrub_icc_profile(&output.stdout)?;
    if expected != output.stdout {
        return Err(MediaError::ResidualMetadata);
    }
    Ok(())
}

fn icc_verification_timeout_secs(
    class: MediaClass,
    image_timeout_secs: u64,
    av_timeout_secs: u64,
) -> u64 {
    match class {
        MediaClass::Image => image_timeout_secs,
        MediaClass::Video | MediaClass::Audio => av_timeout_secs,
    }
}

async fn sanitize_video(
    source: &Path,
    probe: &MediaProbe,
    config: &MediaConfig,
) -> Result<NamedTempFile, MediaError> {
    let output = temp_with_suffix(".mp4")?;
    let can_copy = probe.video_codec.as_deref() == Some("h264")
        && matches!(probe.audio_codec.as_deref(), None | Some("aac"));
    let mut args = common_ffmpeg_input(source);
    args.extend(strings(&[
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-map_metadata",
        "-1",
        "-map_metadata:s",
        "-1",
        "-map_metadata:c",
        "-1",
        "-map_metadata:p",
        "-1",
        "-map_chapters",
        "-1",
        "-sn",
        "-dn",
    ]));
    if can_copy {
        args.extend(strings(&["-c:v", "copy", "-c:a", "copy"]));
    } else {
        args.extend(strings(&["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"]));
        match video_encoder()? {
            "libopenh264" => args.extend(strings(&[
                "-c:v",
                "libopenh264",
                "-b:v",
                "4M",
                "-maxrate",
                "4M",
                "-bufsize",
                "8M",
                "-pix_fmt",
                "yuv420p",
            ])),
            "libx264" => args.extend(strings(&[
                "-c:v", "libx264", "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p",
            ])),
            _ => return Err(MediaError::ToolUnavailable),
        }
        args.extend(strings(&["-c:a", "aac", "-b:a", "192k"]));
    }
    args.extend(strings(&[
        "-bsf:v",
        "filter_units=remove_types=6",
        "-movflags",
        "+faststart",
        "-metadata",
        "encoder=",
    ]));
    args.push(path_string(output.path()));
    require_success(
        &config.ffmpeg_path,
        &args,
        Duration::from_secs(config.av_process_timeout_secs),
    )
    .await?;
    Ok(output)
}

async fn sanitize_audio(
    source: &Path,
    probe: &MediaProbe,
    config: &MediaConfig,
) -> Result<NamedTempFile, MediaError> {
    let (suffix, format) = match probe.ext.as_str() {
        "mp3" => (".mp3", "mp3"),
        "m4a" => (".m4a", "mp4"),
        "aac" => (".aac", "adts"),
        "flac" => (".flac", "flac"),
        "wav" => (".wav", "wav"),
        "ogg" => (".ogg", "ogg"),
        "opus" => (".opus", "opus"),
        _ => return Err(MediaError::UnsupportedMedia(probe.mime.clone())),
    };
    let output = temp_with_suffix(suffix)?;
    let mut args = common_ffmpeg_input(source);
    args.extend(strings(&[
        "-map",
        "0:a:0",
        "-map_metadata",
        "-1",
        "-map_metadata:s",
        "-1",
        "-map_metadata:c",
        "-1",
        "-map_metadata:p",
        "-1",
        "-map_chapters",
        "-1",
        "-vn",
        "-sn",
        "-dn",
        "-c:a",
        "copy",
        "-metadata",
        "encoder=",
        "-f",
        format,
    ]));
    if probe.ext == "mp3" {
        args.extend(strings(&["-id3v2_version", "0", "-write_id3v1", "0"]));
    }
    if probe.ext == "wav" {
        args.extend(strings(&["-fflags", "+bitexact", "-flags:a", "+bitexact"]));
    }
    args.push(path_string(output.path()));
    require_success(
        &config.ffmpeg_path,
        &args,
        Duration::from_secs(config.av_process_timeout_secs),
    )
    .await?;
    Ok(output)
}

async fn verify_forbidden_metadata(
    path: &Path,
    class: MediaClass,
    config: &MediaConfig,
) -> Result<(), MediaError> {
    let selectors = [
        "-GPS*",
        "-Location*",
        "-*Latitude*",
        "-*Longitude*",
        "-*Altitude*",
        "-MakerNotes:All",
        "-Make",
        "-Model",
        "-*SerialNumber*",
        "-OwnerName",
        "-Artist",
        "-Author",
        "-Creator",
        "-Comment",
        "-Description",
        "-Software",
        "-DateTimeOriginal",
        "-CreateDate",
        "-ModifyDate",
        "-MediaCreateDate",
        "-TrackCreateDate",
        "-XMPToolkit",
        "-ThumbnailImage",
        "-PreviewImage",
        "-History*",
        "-DocumentID",
        "-InstanceID",
        "-Lens*",
        "-Camera*",
        "-Copyright",
        "-Title",
        "-Keywords",
        "-Subject",
        "-UserDefinedText",
    ];
    let mut args = strings(&["-api", "LargeFileSupport=1", "-ee", "-j", "-G1", "-s"]);
    args.extend(selectors.iter().map(|value| (*value).to_string()));
    args.push(path_string(path));
    let output = run_tool(
        &config.exiftool_path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(icc_verification_timeout_secs(
            class,
            config.image_process_timeout_secs,
            config.av_process_timeout_secs,
        )),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::SanitizationFailed);
    }
    let documents: Vec<Value> =
        serde_json::from_slice(&output.stdout).map_err(|_| MediaError::SanitizationFailed)?;
    let has_forbidden = documents.iter().any(|document| {
        document
            .as_object()
            .map(|object| {
                object.iter().any(|(key, value)| {
                    if key.ends_with("SourceFile") {
                        return false;
                    }
                    let is_date = key.to_ascii_lowercase().contains("date");
                    let is_zero_date = value.as_str().is_some_and(|value| {
                        value.starts_with("0000:00:00") || value.starts_with("1904:01:01")
                    });
                    !is_date || !is_zero_date
                })
            })
            .unwrap_or(true)
    });
    if has_forbidden {
        return Err(MediaError::ResidualMetadata);
    }
    Ok(())
}

async fn verify_stream_shape(
    probe: &MediaProbe,
    path: &Path,
    config: &MediaConfig,
) -> Result<(), MediaError> {
    // Image structure and geometry/frame limits were already checked by
    // `probe_media`. HEIC/HEIF support is supplied by ExifTool on deployments
    // where FFmpeg cannot expose these still-image containers as streams.
    if probe.class == MediaClass::Image {
        return Ok(());
    }
    let json = ffprobe_json(path, config, false).await?;
    let streams = json["streams"]
        .as_array()
        .ok_or(MediaError::SanitizationFailed)?;
    let video_count = streams
        .iter()
        .filter(|stream| stream["codec_type"] == "video")
        .count();
    let audio_count = streams
        .iter()
        .filter(|stream| stream["codec_type"] == "audio")
        .count();
    let unexpected = streams.iter().any(|stream| {
        !matches!(stream["codec_type"].as_str(), Some("video" | "audio"))
            || stream
                .get("tags")
                .and_then(Value::as_object)
                .is_some_and(|tags| tags.keys().any(|key| !is_structural_tag(key)))
    });
    let valid = match probe.class {
        MediaClass::Image => video_count == 1 && audio_count == 0,
        MediaClass::Video => video_count == 1 && audio_count <= 1,
        MediaClass::Audio => video_count == 0 && audio_count == 1,
    };
    if unexpected || !valid {
        return Err(MediaError::ResidualMetadata);
    }
    Ok(())
}

async fn verify_no_codec_metadata(
    probe: &MediaProbe,
    path: &Path,
    config: &MediaConfig,
) -> Result<(), MediaError> {
    if probe.class != MediaClass::Video || probe.video_codec.as_deref() != Some("h264") {
        return Ok(());
    }

    // H.264 SEI NAL units can contain arbitrary user/device/location payloads
    // that container metadata tools and ffprobe stream tags do not expose.
    // Select only SEI units and require an empty result after sanitization.
    let mut args = common_ffmpeg_input(path);
    args.extend(strings(&[
        "-map",
        "0:v:0",
        "-c:v",
        "copy",
        "-bsf:v",
        "filter_units=pass_types=6",
        "-f",
        "data",
        "pipe:1",
    ]));
    let output = run_tool(
        &config.ffmpeg_path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(config.av_process_timeout_secs),
    )
    .await?;
    if !output.status.success() {
        return Err(MediaError::SanitizationFailed);
    }
    if !output.stdout.is_empty() {
        return Err(MediaError::ResidualMetadata);
    }
    Ok(())
}

fn is_structural_tag(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "language" | "handler_name" | "vendor_id" | "encoder"
    )
}

async fn probe_with_ffprobe(
    path: &Path,
    config: &MediaConfig,
    count_frames: bool,
) -> Result<MediaProbe, MediaError> {
    let json = ffprobe_json(path, config, count_frames).await?;
    let streams = json["streams"]
        .as_array()
        .ok_or(MediaError::SanitizationFailed)?;
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video");
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"] == "audio");
    let format_name = json["format"]["format_name"].as_str().unwrap_or_default();
    let video_codec = video
        .and_then(|stream| stream["codec_name"].as_str())
        .map(str::to_string);
    let audio_codec = audio
        .and_then(|stream| stream["codec_name"].as_str())
        .map(str::to_string);

    let (class, mime, ext) = if video.is_some() {
        if is_still_image_format(format_name) {
            let (mime, ext) = image_format(format_name, video_codec.as_deref())?;
            (MediaClass::Image, mime, ext)
        } else {
            let (mime, ext) = video_format(format_name)?;
            (MediaClass::Video, mime, ext)
        }
    } else if audio.is_some() {
        let (mime, ext) = audio_format(format_name, audio_codec.as_deref())?;
        (MediaClass::Audio, mime, ext)
    } else {
        return Err(MediaError::SanitizationFailed);
    };

    let width = video
        .and_then(|stream| stream["width"].as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let height = video
        .and_then(|stream| stream["height"].as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let duration_secs = json["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            video
                .or(audio)
                .and_then(|stream| stream["duration"].as_str())
                .and_then(|value| value.parse().ok())
        });
    let frame_count = video
        .and_then(|stream| stream["nb_frames"].as_str())
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            video
                .and_then(|stream| stream["nb_read_frames"].as_str())
                .and_then(|value| value.parse().ok())
        });
    Ok(MediaProbe {
        class,
        mime: mime.to_string(),
        ext: ext.to_string(),
        video_codec,
        audio_codec,
        width,
        height,
        duration_secs,
        frame_count,
    })
}

async fn ffprobe_json(
    path: &Path,
    config: &MediaConfig,
    count_frames: bool,
) -> Result<Value, MediaError> {
    let mut args = vec![
        "-v".to_string(),
        "error".to_string(),
        "-protocol_whitelist".to_string(),
        "file,pipe".to_string(),
        "-show_streams".to_string(),
        "-show_format".to_string(),
        "-of".to_string(),
        "json".to_string(),
    ];
    if count_frames {
        args.push("-count_frames".to_string());
    }
    args.push(path_string(path));
    let output = run_tool(
        &config.ffprobe_path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Duration::from_secs(config.av_process_timeout_secs),
    )
    .await?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(MediaError::SanitizationFailed);
    }
    serde_json::from_slice(&output.stdout).map_err(|_| MediaError::SanitizationFailed)
}

fn validate_media_limits(probe: &MediaProbe) -> Result<(), MediaError> {
    match probe.class {
        MediaClass::Image => {
            if probe.ext == "tiff" && probe.frame_count.is_some_and(|frames| frames > 1) {
                return Err(MediaError::UnsupportedMedia("multi-page TIFF".to_string()));
            }
            if let (Some(width), Some(height)) = (probe.width, probe.height) {
                let pixels = u64::from(width) * u64::from(height);
                if pixels > MAX_IMAGE_PIXELS {
                    return Err(MediaError::ImageTooLarge);
                }
                if probe.frame_count.is_some_and(|frames| {
                    frames > MAX_ANIMATION_FRAMES
                        || pixels.saturating_mul(frames) > MAX_ANIMATION_PIXELS
                }) {
                    return Err(MediaError::ImageTooLarge);
                }
            }
        }
        MediaClass::Video => {
            if probe
                .duration_secs
                .is_none_or(|duration| duration <= 0.0 || duration > MAX_VIDEO_DURATION_SECS)
            {
                return Err(MediaError::DurationTooLong);
            }
            if probe.width.is_none_or(|width| width > MAX_VIDEO_WIDTH)
                || probe.height.is_none_or(|height| height > MAX_VIDEO_HEIGHT)
            {
                return Err(MediaError::ResolutionTooHigh);
            }
        }
        MediaClass::Audio => {}
    }
    Ok(())
}

fn iso_bmff_still_image(bytes: &[u8]) -> Option<(&'static str, &'static str, &'static str)> {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return None;
    }
    let box_size = usize::try_from(u32::from_be_bytes(bytes[0..4].try_into().ok()?)).ok()?;
    if box_size < 16 || box_size > bytes.len() {
        return None;
    }

    // Only the declared major brand determines this early classification.
    // Generic MP4 files may advertise mif1/msf1 as compatible brands, and
    // bytes after the ftyp box are not brands at all.
    match &bytes[8..12] {
        b"avif" | b"avis" => Some(("image/avif", "avif", "av1")),
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => {
            Some(("image/heic", "heic", "hevc"))
        }
        _ => None,
    }
}

fn supported_image(mime: &str) -> Option<(&'static str, &'static str)> {
    match mime {
        "image/jpeg" => Some(("image/jpeg", "jpg")),
        "image/png" => Some(("image/png", "png")),
        "image/gif" => Some(("image/gif", "gif")),
        "image/webp" => Some(("image/webp", "webp")),
        "image/tiff" => Some(("image/tiff", "tiff")),
        "image/bmp" | "image/x-ms-bmp" => Some(("image/bmp", "bmp")),
        "image/heic" | "image/heif" => Some(("image/heic", "heic")),
        "image/avif" => Some(("image/avif", "avif")),
        _ => None,
    }
}

fn is_still_image_format(format: &str) -> bool {
    format.split(',').any(|name| {
        matches!(
            name,
            "image2"
                | "jpeg_pipe"
                | "png_pipe"
                | "gif"
                | "webp_pipe"
                | "tiff_pipe"
                | "bmp_pipe"
                | "avif"
                | "heif"
        )
    })
}

fn image_format(
    format: &str,
    codec: Option<&str>,
) -> Result<(&'static str, &'static str), MediaError> {
    if let Some(pair) = codec.and_then(|codec| match codec {
        "mjpeg" => Some(("image/jpeg", "jpg")),
        "png" => Some(("image/png", "png")),
        "gif" => Some(("image/gif", "gif")),
        "webp" => Some(("image/webp", "webp")),
        "tiff" => Some(("image/tiff", "tiff")),
        "bmp" => Some(("image/bmp", "bmp")),
        "av1" => Some(("image/avif", "avif")),
        _ => None,
    }) {
        return Ok(pair);
    }
    Err(MediaError::UnsupportedMedia(format.to_string()))
}

fn video_format(format: &str) -> Result<(&'static str, &'static str), MediaError> {
    if format.contains("matroska") || format.contains("webm") {
        if format.contains("webm") {
            Ok(("video/webm", "webm"))
        } else {
            Ok(("video/x-matroska", "mkv"))
        }
    } else if format.contains("mov") || format.contains("mp4") {
        Ok(("video/mp4", "mp4"))
    } else {
        Err(MediaError::UnsupportedMedia(format.to_string()))
    }
}

fn audio_format(
    format: &str,
    codec: Option<&str>,
) -> Result<(&'static str, &'static str), MediaError> {
    if format.contains("mp3") {
        Ok(("audio/mpeg", "mp3"))
    } else if format.contains("mov") || format.contains("mp4") {
        Ok(("audio/mp4", "m4a"))
    } else if format.contains("aac") {
        Ok(("audio/aac", "aac"))
    } else if format.contains("flac") {
        Ok(("audio/flac", "flac"))
    } else if format.contains("wav") {
        Ok(("audio/wav", "wav"))
    } else if format.contains("ogg") {
        if codec == Some("opus") {
            Ok(("audio/opus", "opus"))
        } else {
            Ok(("audio/ogg", "ogg"))
        }
    } else {
        Err(MediaError::UnsupportedMedia(format.to_string()))
    }
}

fn common_ffmpeg_input(source: &Path) -> Vec<String> {
    let mut args = strings(&[
        "-nostdin",
        "-v",
        "error",
        "-y",
        "-protocol_whitelist",
        "file,pipe",
        "-i",
    ]);
    args.push(path_string(source));
    args
}

async fn require_success(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<(), MediaError> {
    let output = run_tool(
        program,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        timeout,
    )
    .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(MediaError::SanitizationFailed)
    }
}

async fn run_tool(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<std::process::Output, MediaError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            MediaError::ToolUnavailable
        } else {
            MediaError::Io(error.to_string())
        }
    })?;
    let stdout = child.stdout.take().ok_or(MediaError::Internal)?;
    let stderr = child.stderr.take().ok_or(MediaError::Internal)?;
    let stdout_task = tokio::spawn(read_bounded_output(stdout));
    let stderr_task = tokio::spawn(read_bounded_output(stderr));
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(result) => result.map_err(|error| MediaError::Io(error.to_string()))?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(MediaError::SanitizationFailed);
        }
    };
    let stdout = stdout_task.await.map_err(|_| MediaError::Internal)??;
    let stderr = stderr_task.await.map_err(|_| MediaError::Internal)??;
    if stdout.len() > MAX_TOOL_OUTPUT || stderr.len() > MAX_TOOL_OUTPUT {
        return Err(MediaError::SanitizationFailed);
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

async fn read_bounded_output(
    reader: impl tokio::io::AsyncRead + Unpin,
) -> Result<Vec<u8>, MediaError> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_TOOL_OUTPUT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    Ok(bytes)
}

async fn read_sniff(path: &Path) -> Result<Vec<u8>, MediaError> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    let mut bytes = vec![0_u8; 4096];
    let read = file
        .read(&mut bytes)
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?;
    bytes.truncate(read);
    Ok(bytes)
}

fn temp_with_suffix(suffix: &str) -> Result<NamedTempFile, MediaError> {
    Builder::new()
        .prefix("buzz-sanitized-")
        .suffix(suffix)
        .tempfile()
        .map_err(|error| MediaError::Io(error.to_string()))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_icc_profile() -> Vec<u8> {
        let private_description =
            canonical_icc_text(b"desc", b"desc", "Alice's iPhone profile").unwrap();
        let private_copyright = canonical_icc_text(b"cprt", b"text", "Copyright Alice").unwrap();
        let private_model = canonical_icc_text(b"desc", b"desc", "Alice's iPhone 17").unwrap();
        let mut xyz = Vec::from(&b"XYZ \0\0\0\0"[..]);
        xyz.extend_from_slice(&[0; 12]);
        let tags = [
            (*b"desc", private_description),
            (*b"cprt", private_copyright),
            (*b"dmdd", private_model),
            (*b"rXYZ", xyz),
        ];

        let table_end = 132 + tags.len() * 12;
        let mut profile = vec![0_u8; table_end];
        profile[4..8].copy_from_slice(b"Buzz");
        profile[8..12].copy_from_slice(&[4, 0x30, 0, 0]);
        profile[12..16].copy_from_slice(b"mntr");
        profile[16..20].copy_from_slice(b"RGB ");
        profile[20..24].copy_from_slice(b"XYZ ");
        profile[24..36].fill(1);
        profile[36..40].copy_from_slice(b"acsp");
        profile[40..44].copy_from_slice(b"APPL");
        profile[48..52].copy_from_slice(b"APPL");
        profile[52..56].copy_from_slice(b"iPhn");
        profile[80..84].copy_from_slice(b"Buzz");
        profile[84..100].fill(0x5a);
        profile[128..132].copy_from_slice(&(tags.len() as u32).to_be_bytes());
        for (index, (signature, data)) in tags.into_iter().enumerate() {
            while !profile.len().is_multiple_of(4) {
                profile.push(0);
            }
            let offset = profile.len();
            profile.extend_from_slice(&data);
            let entry = 132 + index * 12;
            profile[entry..entry + 4].copy_from_slice(&signature);
            profile[entry + 4..entry + 8].copy_from_slice(&(offset as u32).to_be_bytes());
            profile[entry + 8..entry + 12].copy_from_slice(&(data.len() as u32).to_be_bytes());
        }
        let size = profile.len() as u32;
        profile[0..4].copy_from_slice(&size.to_be_bytes());
        profile
    }

    #[test]
    fn supported_format_tables_are_bounded() {
        assert_eq!(supported_image("image/jpeg"), Some(("image/jpeg", "jpg")));
        assert!(supported_image("image/svg+xml").is_none());
        assert_eq!(audio_format("ogg", Some("opus")).unwrap().1, "opus");
        assert!(video_format("avi").is_err());
        assert_eq!(
            iso_bmff_still_image(b"\0\0\0\x14ftypheic\0\0\0\0heic"),
            Some(("image/heic", "heic", "hevc"))
        );
        assert_eq!(
            iso_bmff_still_image(b"\0\0\0\x14ftypmp42\0\0\0\0mif1"),
            None
        );
        assert_eq!(
            iso_bmff_still_image(b"\0\0\0\x10ftypmp42\0\0\0\0mif1"),
            None
        );
        let args = common_ffmpeg_input(Path::new("fixture.mp4"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-protocol_whitelist", "file,pipe"]));
    }

    #[test]
    fn structural_tag_allowlist_is_narrow() {
        assert!(is_structural_tag("language"));
        assert!(!is_structural_tag("location"));
        assert!(!is_structural_tag("title"));
        assert!(valid_icc_color_space(b"1CLR"));
    }

    #[test]
    fn icc_scrubber_preserves_color_data_but_removes_identity() {
        let source = test_icc_profile();
        let scrubbed = scrub_icc_profile(&source).expect("scrub ICC profile");
        assert_eq!(scrub_icc_profile(&scrubbed).unwrap(), scrubbed);
        assert!(!scrubbed
            .windows("Alice".len())
            .any(|window| window == b"Alice"));
        assert!(scrubbed
            .windows(ICC_CANONICAL_DESCRIPTION.len())
            .any(|window| window == ICC_CANONICAL_DESCRIPTION.as_bytes()));
        assert!(scrubbed
            .windows(ICC_CANONICAL_COPYRIGHT.len())
            .any(|window| window == ICC_CANONICAL_COPYRIGHT.as_bytes()));
        assert!(scrubbed.windows(4).any(|window| window == b"rXYZ"));
        assert!(!scrubbed.windows(4).any(|window| window == b"dmdd"));
        for range in [4..8, 10..12, 24..36, 40..64, 80..100, 100..128] {
            assert!(scrubbed[range].iter().all(|byte| *byte == 0));
        }
    }

    #[test]
    fn icc_header_reserved_channels_are_canonicalized() {
        let mut source = test_icc_profile();
        source[10..12].fill(0x5a);
        source[44..64].fill(0x5a);
        source[64..68].copy_from_slice(&2_u32.to_be_bytes());
        source[68..80].fill(0x5a);
        let scrubbed = scrub_icc_profile(&source).unwrap();
        assert_eq!(&scrubbed[8..12], &[4, 0x30, 0, 0]);
        assert!(scrubbed[44..64].iter().all(|byte| *byte == 0));
        assert_eq!(&scrubbed[64..68], &2_u32.to_be_bytes());
        assert_eq!(
            &scrubbed[68..80],
            &[0x00, 0x00, 0xf6, 0xd6, 0x00, 0x01, 0, 0, 0, 0, 0xd3, 0x2d]
        );
    }

    #[test]
    fn icc_verification_uses_media_class_timeout() {
        assert_eq!(
            icc_verification_timeout_secs(MediaClass::Image, 120, 600),
            120
        );
        assert_eq!(
            icc_verification_timeout_secs(MediaClass::Audio, 120, 600),
            600
        );
        assert_eq!(
            icc_verification_timeout_secs(MediaClass::Video, 120, 600),
            600
        );
    }

    #[test]
    fn icc_scrubber_rejects_private_tag_payloads() {
        let mut source = test_icc_profile();
        let tag_count = read_icc_u32(&source, 128).unwrap() as usize;
        source[132 + (tag_count - 1) * 12..136 + (tag_count - 1) * 12].copy_from_slice(b"priv");
        assert!(matches!(
            scrub_icc_profile(&source),
            Err(MediaError::SanitizationFailed)
        ));
    }

    #[test]
    fn icc_scrubber_rejects_duplicate_signatures() {
        let mut source = test_icc_profile();
        source[168..172].copy_from_slice(b"desc");
        assert!(matches!(
            scrub_icc_profile(&source),
            Err(MediaError::SanitizationFailed)
        ));
    }

    #[test]
    fn icc_scrubber_drops_common_apple_display_profile_tags() {
        for private_tag in [*b"aarg", *b"aabg", *b"aagg", *b"vcgt", *b"ndin", *b"vcgp"] {
            let mut source = test_icc_profile();
            source[168..172].copy_from_slice(&private_tag);
            let scrubbed = scrub_icc_profile(&source).expect("scrub Apple display profile");
            assert!(!scrubbed
                .windows(private_tag.len())
                .any(|window| window == private_tag));
        }
    }

    #[test]
    fn icc_scrubber_canonicalizes_tag_order() {
        let source = test_icc_profile();
        let mut reordered = source.clone();
        let tag_count = read_icc_u32(&source, 128).unwrap() as usize;
        for index in 0..tag_count {
            let source_entry = 132 + index * 12;
            let target_entry = 132 + (tag_count - index - 1) * 12;
            reordered[target_entry..target_entry + 12]
                .copy_from_slice(&source[source_entry..source_entry + 12]);
        }
        assert_eq!(
            scrub_icc_profile(&source).unwrap(),
            scrub_icc_profile(&reordered).unwrap()
        );
    }

    #[test]
    fn icc_rendering_payloads_reject_trailing_private_bytes() {
        let mut curve = Vec::from(&b"curv\0\0\0\0\0\0\0\x01\x01\0"[..]);
        assert!(canonical_icc_rendering_tag(b"rTRC", &curve).is_ok());
        curve.extend_from_slice(&[0; 2]);
        assert_eq!(
            canonical_icc_rendering_tag(b"rTRC", &curve).unwrap(),
            &curve[..14]
        );
        curve.extend_from_slice(b"Alice in Chicago");
        assert!(matches!(
            canonical_icc_rendering_tag(b"rTRC", &curve),
            Err(MediaError::SanitizationFailed)
        ));
    }

    #[test]
    fn icc_parametric_curves_reject_reserved_bytes() {
        let mut curve = Vec::from(&b"para\0\0\0\0\0\0\0\0\0\x01\0\0"[..]);
        assert!(canonical_icc_rendering_tag(b"rTRC", &curve).is_ok());
        curve[10] = 0x5a;
        assert!(canonical_icc_rendering_tag(b"rTRC", &curve).is_err());
    }

    #[test]
    fn icc_scrubber_pads_total_profile_size() {
        let mut source = test_icc_profile();
        let tag_entry = 168;
        let data_offset = read_icc_u32(&source, tag_entry + 4).unwrap() as usize;
        source[tag_entry..tag_entry + 4].copy_from_slice(b"rTRC");
        source[tag_entry + 8..tag_entry + 12].copy_from_slice(&14_u32.to_be_bytes());
        source[data_offset..data_offset + 14].copy_from_slice(b"curv\0\0\0\0\0\0\0\x01\x01\0");
        let scrubbed = scrub_icc_profile(&source).unwrap();
        assert!(scrubbed.len().is_multiple_of(4));
        assert_eq!(read_icc_u32(&scrubbed, 0).unwrap() as usize, scrubbed.len());
    }

    #[test]
    fn icc_colorant_order_uses_specified_type_and_exact_length() {
        let mut colorant_order = Vec::from(&b"clro\0\0\0\0\0\0\0\x03"[..]);
        colorant_order.extend_from_slice(&[0, 1, 2]);
        assert_eq!(
            canonical_icc_rendering_tag(b"clro", &colorant_order).unwrap(),
            colorant_order
        );

        colorant_order[0..4].copy_from_slice(b"ui08");
        assert!(canonical_icc_rendering_tag(b"clro", &colorant_order).is_err());
    }

    #[test]
    fn icc_v2_description_has_complete_script_code_fields() {
        let description = canonical_icc_text(b"desc", b"desc", "profile").unwrap();
        assert_eq!(description.len(), 91 + "profile".len());
        let script_code_start = 21 + "profile".len();
        assert_eq!(
            &description[script_code_start..script_code_start + 70],
            &[0; 70]
        );
    }

    #[test]
    fn nonredistributable_ffmpeg_builds_are_rejected() {
        assert!(build_is_nonredistributable(
            "configuration: --enable-gpl --enable-nonfree --enable-libx264"
        ));
        assert!(!build_is_nonredistributable(
            "configuration: --disable-autodetect --enable-libopenh264 --enable-shared"
        ));
    }

    #[test]
    fn openh264_is_preferred_without_requiring_it_from_operators() {
        let both = " V....D libx264 H.264 / AVC\n V....D libopenh264 OpenH264 H.264";
        assert_eq!(select_video_encoder(both), Some("libopenh264"));
        assert_eq!(
            select_video_encoder(" V....D libx264 H.264 / AVC"),
            Some("libx264")
        );
        assert_eq!(select_video_encoder(" A..... aac AAC"), None);
    }

    #[test]
    fn video_and_animation_limits_fail_closed() {
        let mut probe = MediaProbe {
            class: MediaClass::Video,
            mime: "video/mp4".to_string(),
            ext: "mp4".to_string(),
            video_codec: Some("h264".to_string()),
            audio_codec: Some("aac".to_string()),
            width: Some(3840),
            height: Some(2160),
            duration_secs: Some(600.0),
            frame_count: Some(18_000),
        };
        assert!(validate_media_limits(&probe).is_ok());
        probe.duration_secs = Some(600.1);
        assert!(matches!(
            validate_media_limits(&probe),
            Err(MediaError::DurationTooLong)
        ));
        probe.class = MediaClass::Image;
        probe.width = Some(1_000);
        probe.height = Some(1_000);
        probe.duration_secs = None;
        probe.frame_count = Some(1_001);
        assert!(matches!(
            validate_media_limits(&probe),
            Err(MediaError::ImageTooLarge)
        ));
        probe.ext = "tiff".to_string();
        probe.frame_count = Some(2);
        assert!(matches!(
            validate_media_limits(&probe),
            Err(MediaError::UnsupportedMedia(format)) if format == "multi-page TIFF"
        ));
    }
}
