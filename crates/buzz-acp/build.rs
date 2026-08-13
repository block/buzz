#[path = "src/heartbeat_capability_constants.rs"]
mod heartbeat_capability_constants;

use std::path::PathBuf;

use heartbeat_capability_constants::{BUILD_CAPABILITY, KIND, PROTOCOL_VERSION};

const INFO_PLIST_FILENAME: &str = "buzz-acp-heartbeat-capability-Info.plist";
const INFO_PLIST_KEY: &str = "BuzzHeartbeatPreflightCapability";

fn capability_attestation() -> String {
    format!("{KIND}/v{PROTOCOL_VERSION}/{BUILD_CAPABILITY}")
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn info_plist(attestation: &str) -> String {
    let attestation = escape_xml_text(attestation);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>{INFO_PLIST_KEY}</key>\n\
         \t<string>{attestation}</string>\n\
         </dict>\n\
         </plist>\n"
    )
}

fn main() {
    println!("cargo:rerun-if-changed=src/heartbeat_capability_constants.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let out_dir = std::env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR");
    let plist_path = PathBuf::from(out_dir).join(INFO_PLIST_FILENAME);
    std::fs::write(&plist_path, info_plist(&capability_attestation()))
        .expect("write buzz-acp heartbeat capability Info.plist");

    let plist_path = plist_path
        .to_str()
        .expect("heartbeat capability Info.plist path must be UTF-8");
    println!("cargo:rustc-link-arg-bin=buzz-acp=-Wl,-sectcreate,__TEXT,__info_plist,{plist_path}");
}
