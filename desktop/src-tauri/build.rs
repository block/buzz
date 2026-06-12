fn main() {
    println!("cargo:rerun-if-env-changed=BUZZ_RELAY_URL");
    println!("cargo:rerun-if-env-changed=BUZZ_RELAY_HTTP");
    println!("cargo:rerun-if-env-changed=BUZZ_UPDATER_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=BUZZ_UPDATER_ENDPOINT");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_DATABRICKS_HOST");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_DATABRICKS_MODEL");
    println!("cargo:rustc-check-cfg=cfg(buzz_updater_enabled)");

    if let Ok(relay_url) = std::env::var("BUZZ_RELAY_URL") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_RELAY_URL={relay_url}");
    }

    if let Ok(relay_http) = std::env::var("BUZZ_RELAY_HTTP") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_RELAY_HTTP={relay_http}");
    }

    if let Ok(host) = std::env::var("BUZZ_BUILD_DATABRICKS_HOST") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_DATABRICKS_HOST={host}");
    }

    if let Ok(model) = std::env::var("BUZZ_BUILD_DATABRICKS_MODEL") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_DATABRICKS_MODEL={model}");
    }

    let updater_public_key = std::env::var("BUZZ_UPDATER_PUBLIC_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let updater_endpoint = std::env::var("BUZZ_UPDATER_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if updater_public_key.is_some() && updater_endpoint.is_some() {
        println!("cargo:rustc-cfg=buzz_updater_enabled");
    }

    tauri_build::build()
}
