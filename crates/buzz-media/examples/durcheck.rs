use std::path::Path;
use std::process::Command;

use buzz_media::config::MediaConfig;
use buzz_media::validation::validate_video_file;

fn relay_media_config() -> MediaConfig {
    MediaConfig {
        s3_endpoint: "http://localhost:9000".to_string(),
        s3_access_key: "buzz_dev".to_string(),
        s3_secret_key: "buzz_dev_secret".to_string(),
        s3_bucket: "buzz-media".to_string(),
        s3_region: "us-east-1".to_string(),
        s3_addressing_style: buzz_media::config::S3AddressingStyle::default(),
        max_image_bytes: 50 * 1024 * 1024,
        max_gif_bytes: 50 * 1024 * 1024,
        max_file_bytes: 100 * 1024 * 1024,
        max_video_bytes: 500 * 1024 * 1024,
        public_base_url: "http://localhost:3000/media".to_string(),
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    }
}

fn main() {
    let wav = std::env::args().nth(1).expect("wav");
    let out = std::env::args().nth(2).expect("out");
    let r: u32 = std::env::args().nth(3).expect("rate").parse().unwrap();

    let status = Command::new("ffmpeg")
        .args([
            "-y", "-nostdin", "-loglevel", "error", "-f", "lavfi", "-i",
            &format!("color=c=black:s=16x16:r={r}"),
        ])
        .arg("-i").arg(&wav)
        .args([
            "-map", "0:v:0", "-map", "1:a:0", "-shortest", "-map_metadata", "-1",
            "-map_chapters", "-1", "-sn", "-dn", "-fflags", "+bitexact",
            "-flags:v", "+bitexact", "-flags:a", "+bitexact", "-c:v", "libx264",
            "-preset", "ultrafast", "-tune", "stillimage", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart", "-metadata",
            "encoder=",
        ])
        .arg(&out)
        .status()
        .expect("ffmpeg");
    assert!(status.success());

    let meta = validate_video_file(Path::new(&out), &relay_media_config())
        .unwrap_or_else(|e| panic!("relay REJECTED {out}: {e}"));

    // spoken length from the wav: parse RIFF chunks rather than assuming the
    // `data` chunk sits at a fixed offset (ffmpeg inserts a LIST chunk).
    let wav_bytes = std::fs::read(&wav).expect("read wav");
    let mut rate = 0u32;
    let mut data_len = 0u32;
    let mut pos = 12usize; // past "RIFF" + size + "WAVE"
    while pos + 8 <= wav_bytes.len() {
        let id = &wav_bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            wav_bytes[pos + 4],
            wav_bytes[pos + 5],
            wav_bytes[pos + 6],
            wav_bytes[pos + 7],
        ]) as usize;
        if id == b"fmt " {
            rate = u32::from_le_bytes([
                wav_bytes[pos + 12],
                wav_bytes[pos + 13],
                wav_bytes[pos + 14],
                wav_bytes[pos + 15],
            ]);
        }
        if id == b"data" {
            data_len = size as u32;
        }
        pos += 8 + size + (size & 1);
    }
    let seconds = data_len as f64 / 2.0 / rate as f64;

    let drift = (seconds - meta.duration_secs).abs();
    println!(
        "r={r:<3} spoken={seconds:.3}  stored={:.3}  drift={drift:.3}  dim={}x{}  audio={}  \
         -> {}",
        meta.duration_secs,
        meta.width,
        meta.height,
        meta.has_audio,
        if drift <= 0.1 { "PASS" } else { "FAIL" }
    );
}
