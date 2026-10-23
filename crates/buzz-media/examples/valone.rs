use buzz_media::config::MediaConfig;
use buzz_media::validation::validate_video_file;

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let cfg = MediaConfig {
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
    };
    match validate_video_file(std::path::Path::new(&path), &cfg) {
        Ok(m) => println!(
            "RELAY ACCEPTS {}  stored_duration={:.3} dim={}x{} audio={}",
            path, m.duration_secs, m.width, m.height, m.has_audio
        ),
        Err(e) => println!("RELAY REJECTS {}: {e}", path),
    }
}
