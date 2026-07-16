use std::path::Path;
use std::process::Command;

use base64::Engine;
use buzz_media::sanitize::{probe_media, sanitize, validate_toolchain, MediaClass};
use buzz_media::MediaConfig;
use sha2::{Digest, Sha256};

const ICC_CANONICAL_DESCRIPTION: &str = "Sanitized color profile";
const ICC_CANONICAL_COPYRIGHT: &str = "Sanitized by Buzz";
const PRIVATE_H264_SEI: &[u8] = b"GPS_PRIVATE_ALICE_IPHONE";

fn config() -> MediaConfig {
    MediaConfig {
        s3_endpoint: String::new(),
        s3_access_key: String::new(),
        s3_secret_key: String::new(),
        s3_bucket: String::new(),
        s3_region: "us-east-1".to_string(),
        max_image_bytes: 50 * 1024 * 1024,
        max_gif_bytes: 10 * 1024 * 1024,
        max_video_bytes: 500 * 1024 * 1024,
        max_audio_bytes: 100 * 1024 * 1024,
        max_file_bytes: 100 * 1024 * 1024,
        public_base_url: "http://localhost:3000/media".to_string(),
        exiftool_path: std::env::var("BUZZ_EXIFTOOL_PATH")
            .unwrap_or_else(|_| "exiftool".to_string()),
        ffmpeg_path: std::env::var("BUZZ_FFMPEG_PATH").unwrap_or_else(|_| "ffmpeg".to_string()),
        ffprobe_path: std::env::var("BUZZ_FFPROBE_PATH").unwrap_or_else(|_| "ffprobe".to_string()),
        image_process_timeout_secs: 120,
        av_process_timeout_secs: 600,
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    }
}

fn run(program: &str, args: &[&str]) {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {program}: {error}"));
    assert!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn path(path: &Path) -> &str {
    path.to_str().expect("fixture path must be UTF-8")
}

fn ffmpeg(config: &MediaConfig, args: &[&str]) {
    let mut full = vec!["-nostdin", "-v", "error", "-y"];
    full.extend_from_slice(args);
    let generator =
        std::env::var("BUZZ_FIXTURE_FFMPEG_PATH").unwrap_or_else(|_| config.ffmpeg_path.clone());
    run(&generator, &full);
}

fn add_private_metadata(config: &MediaConfig, fixture: &Path) {
    run(
        &config.exiftool_path,
        &[
            "-overwrite_original",
            "-GPSLatitude=41.8781",
            "-GPSLatitudeRef=N",
            "-GPSLongitude=87.6298",
            "-GPSLongitudeRef=W",
            "-Make=BuzzFixture",
            "-Model=ComplianceCamera",
            "-Artist=Buzz Compliance Fixture",
            "-Comment=Synthetic location fixture",
            "-Title=Synthetic location fixture",
            path(fixture),
        ],
    );
}

fn icc_text(value: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(9 + value.len());
    data.extend_from_slice(b"text");
    data.extend_from_slice(&[0; 4]);
    data.extend_from_slice(value.as_bytes());
    data.push(0);
    data
}

fn icc_description(value: &str) -> Vec<u8> {
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
    data
}

fn descriptive_icc_profile() -> Vec<u8> {
    let tags = [
        (*b"desc", icc_description("Alice's iPhone color profile")),
        (*b"cprt", icc_text("Copyright Alice")),
        (*b"dmdd", icc_description("Alice's iPhone 17")),
        (
            *b"rXYZ",
            Vec::from(&b"XYZ \0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"[..]),
        ),
    ];
    let mut profile = vec![0_u8; 132 + tags.len() * 12];
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

fn add_descriptive_icc(config: &MediaConfig, fixture: &Path) {
    let profile = fixture.with_extension("icc");
    std::fs::write(&profile, descriptive_icc_profile()).expect("write synthetic ICC profile");
    let assignment = format!("-ICC_Profile<={}", path(&profile));
    run(
        &config.exiftool_path,
        &["-overwrite_original", &assignment, path(fixture)],
    );
}

fn forbidden_metadata(
    config: &MediaConfig,
    fixture: &Path,
) -> serde_json::Map<String, serde_json::Value> {
    let output = Command::new(&config.exiftool_path)
        .args([
            "-j",
            "-G1",
            "-s",
            "-GPS*",
            "-Location*",
            "-Make",
            "-Model",
            "-Artist",
            "-Comment",
            "-Title",
            "-UserDefinedText",
            "-ProfileDescription",
            "-ProfileCopyright",
            "-DeviceMfgDesc",
            "-DeviceModelDesc",
            path(fixture),
        ])
        .output()
        .expect("run exiftool metadata oracle");
    assert!(output.status.success(), "metadata oracle failed");
    let values: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("ExifTool JSON");
    let mut object = values[0].as_object().expect("ExifTool object").clone();
    object.retain(|key, value| {
        if key.ends_with("SourceFile") {
            return true;
        }
        !matches!(
            (key.rsplit(':').next(), value.as_str()),
            (Some("ProfileDescription"), Some(ICC_CANONICAL_DESCRIPTION))
                | (Some("ProfileCopyright"), Some(ICC_CANONICAL_COPYRIGHT))
        )
    });
    object
}

fn source_hash(path: &Path) -> String {
    hex::encode(Sha256::digest(std::fs::read(path).expect("read fixture")))
}

fn synchsafe(value: usize) -> [u8; 4] {
    [
        ((value >> 21) & 0x7f) as u8,
        ((value >> 14) & 0x7f) as u8,
        ((value >> 7) & 0x7f) as u8,
        (value & 0x7f) as u8,
    ]
}

fn id3_text_frame(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(10 + payload.len());
    frame.extend_from_slice(id);
    frame.extend_from_slice(&synchsafe(payload.len()));
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(payload);
    frame
}

fn prepend_aac_id3(fixture: &Path) {
    let title = id3_text_frame(b"TIT2", b"\x03Synthetic location fixture");
    let location = id3_text_frame(b"TXXX", b"\x03LOCATION\x0041.8781,-87.6298");
    let mut frames = Vec::new();
    frames.extend_from_slice(&title);
    frames.extend_from_slice(&location);

    let audio = std::fs::read(fixture).expect("read AAC fixture");
    let mut tagged = Vec::with_capacity(10 + frames.len() + audio.len());
    tagged.extend_from_slice(b"ID3\x04\x00\x00");
    tagged.extend_from_slice(&synchsafe(frames.len()));
    tagged.extend_from_slice(&frames);
    tagged.extend_from_slice(&audio);
    std::fs::write(fixture, tagged).expect("write ID3-tagged AAC fixture");
}

async fn assert_sanitized(
    config: &MediaConfig,
    source: &Path,
    expected_class: MediaClass,
    require_source_metadata: bool,
) {
    let source_metadata = forbidden_metadata(config, source);
    if require_source_metadata {
        assert!(
            source_metadata
                .keys()
                .any(|key| !key.ends_with("SourceFile")),
            "{} must prove it contains private metadata before sanitizing",
            source.display()
        );
    }
    let bytes = std::fs::read(source).expect("read source");
    let has_private_h264_sei = bytes
        .windows(PRIVATE_H264_SEI.len())
        .any(|window| window == PRIVATE_H264_SEI);
    let probe = probe_media(source, &bytes[..bytes.len().min(4096)], config)
        .await
        .expect("probe source")
        .expect("fixture must be recognized media");
    assert_eq!(probe.class, expected_class);
    let input_hash = source_hash(source);
    let output = sanitize(source, &probe, config)
        .await
        .unwrap_or_else(|error| panic!("sanitize fixture {}: {error:?}", source.display()));
    assert_eq!(output.probe.class, expected_class);
    assert_ne!(input_hash, source_hash(output.file.path()));
    if has_private_h264_sei {
        let output_bytes = std::fs::read(output.file.path()).expect("read sanitized fixture");
        assert!(
            !output_bytes
                .windows(PRIVATE_H264_SEI.len())
                .any(|window| window == PRIVATE_H264_SEI),
            "{} retained private H.264 SEI user data",
            source.display()
        );
    }
    let residual = forbidden_metadata(config, output.file.path());
    assert!(
        residual.keys().all(|key| key.ends_with("SourceFile")),
        "{} retained forbidden metadata keys: {:?}",
        source.display(),
        residual.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
#[ignore = "mandatory via `just media-compliance-test`"]
async fn realistic_media_matrix_strips_location_and_descriptive_metadata() {
    let config = config();
    validate_toolchain(&config)
        .await
        .expect("FFmpeg, ffprobe, and ExifTool are mandatory");
    let temp = tempfile::tempdir().expect("fixture directory");

    let image_specs = [
        (
            "jpg",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x64:rate=1",
                "-frames:v",
                "1",
            ],
        ),
        (
            "png",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x64:rate=1",
                "-frames:v",
                "1",
            ],
        ),
        (
            "gif",
            vec!["-f", "lavfi", "-i", "testsrc2=size=96x64:rate=2", "-t", "1"],
        ),
        (
            "webp",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x64:rate=1",
                "-frames:v",
                "1",
            ],
        ),
        (
            "tiff",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x64:rate=1",
                "-frames:v",
                "1",
            ],
        ),
        (
            "bmp",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x64:rate=1",
                "-frames:v",
                "1",
            ],
        ),
        (
            "avif",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=96x64:rate=1",
                "-frames:v",
                "1",
                "-c:v",
                "libaom-av1",
                "-still-picture",
                "1",
            ],
        ),
    ];
    for (ext, mut args) in image_specs {
        let fixture = temp.path().join(format!("location.{ext}"));
        args.push(path(&fixture));
        ffmpeg(&config, &args);
        if ext != "bmp" {
            add_private_metadata(&config, &fixture);
        }
        if ext == "jpg" {
            add_descriptive_icc(&config, &fixture);
        }
        assert_sanitized(&config, &fixture, MediaClass::Image, ext != "bmp").await;
    }

    let heic = temp.path().join("location.heic");
    let encoded = include_str!("fixtures/tiny.heic.b64").trim();
    std::fs::write(
        &heic,
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("decode HEIC fixture"),
    )
    .expect("write HEIC fixture");
    add_private_metadata(&config, &heic);
    assert_sanitized(&config, &heic, MediaClass::Image, true).await;

    for ext in ["mp4", "mov", "webm", "mkv"] {
        let fixture = temp.path().join(format!("location.{ext}"));
        let video_source = if ext == "webm" {
            "testsrc=size=95x63:rate=10:duration=1,format=yuv444p"
        } else {
            "testsrc2=size=96x64:rate=10:duration=1"
        };
        let mut args = vec![
            "-f",
            "lavfi",
            "-i",
            video_source,
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-shortest",
            "-metadata",
            "location=+41.8781-087.6298/",
            "-metadata",
            "title=Synthetic location fixture",
        ];
        if matches!(ext, "webm" | "mkv") {
            args.extend(["-c:v", "libvpx-vp9", "-c:a", "libopus"]);
        } else {
            args.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
        }
        args.push(path(&fixture));
        ffmpeg(&config, &args);
        if ext == "mp4" {
            let with_sei = temp.path().join("location-with-private-sei.mp4");
            ffmpeg(
                &config,
                &[
                    "-i",
                    path(&fixture),
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0?",
                    "-c",
                    "copy",
                    "-bsf:v",
                    "h264_metadata=sei_user_data=00112233-4455-6677-8899-aabbccddeeff+GPS_PRIVATE_ALICE_IPHONE",
                    path(&with_sei),
                ],
            );
            std::fs::rename(&with_sei, &fixture).expect("install private-SEI fixture");
            assert!(
                std::fs::read(&fixture)
                    .expect("read private-SEI fixture")
                    .windows(PRIVATE_H264_SEI.len())
                    .any(|window| window == PRIVATE_H264_SEI),
                "MP4 fixture must contain private H.264 SEI before sanitizing"
            );
        }
        assert_sanitized(&config, &fixture, MediaClass::Video, true).await;
    }

    let audio_specs = [
        ("mp3", "libmp3lame"),
        ("m4a", "aac"),
        ("aac", "aac"),
        ("flac", "flac"),
        ("wav", "pcm_s16le"),
        ("ogg", "libvorbis"),
        ("opus", "libopus"),
    ];
    for (ext, codec) in audio_specs {
        let fixture = temp.path().join(format!("location.{ext}"));
        ffmpeg(
            &config,
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-c:a",
                codec,
                "-metadata",
                "LOCATION=41.8781,-87.6298",
                "-metadata",
                "title=Synthetic location fixture",
                path(&fixture),
            ],
        );
        if ext == "aac" {
            prepend_aac_id3(&fixture);
        }
        assert_sanitized(&config, &fixture, MediaClass::Audio, true).await;
    }
}
