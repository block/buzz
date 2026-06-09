fn main() {
    println!("cargo:rerun-if-env-changed=SPROUT_RELAY_URL");
    println!("cargo:rerun-if-env-changed=SPROUT_RELAY_HTTP");
    println!("cargo:rerun-if-env-changed=SPROUT_UPDATER_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=SPROUT_UPDATER_ENDPOINT");
    println!("cargo:rerun-if-env-changed=SPROUT_BUILD_DATABRICKS_HOST");
    println!("cargo:rerun-if-env-changed=SPROUT_BUILD_DATABRICKS_MODEL");
    println!("cargo:rustc-check-cfg=cfg(sprout_updater_enabled)");

    if let Ok(relay_url) = std::env::var("SPROUT_RELAY_URL") {
        println!("cargo:rustc-env=SPROUT_DESKTOP_BUILD_RELAY_URL={relay_url}");
    }

    if let Ok(relay_http) = std::env::var("SPROUT_RELAY_HTTP") {
        println!("cargo:rustc-env=SPROUT_DESKTOP_BUILD_RELAY_HTTP={relay_http}");
    }

    if let Ok(host) = std::env::var("SPROUT_BUILD_DATABRICKS_HOST") {
        println!("cargo:rustc-env=SPROUT_DESKTOP_BUILD_DATABRICKS_HOST={host}");
    }

    if let Ok(model) = std::env::var("SPROUT_BUILD_DATABRICKS_MODEL") {
        println!("cargo:rustc-env=SPROUT_DESKTOP_BUILD_DATABRICKS_MODEL={model}");
    }

    let updater_public_key = std::env::var("SPROUT_UPDATER_PUBLIC_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let updater_endpoint = std::env::var("SPROUT_UPDATER_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if updater_public_key.is_some() && updater_endpoint.is_some() {
        println!("cargo:rustc-cfg=sprout_updater_enabled");
    }

    tauri_build::build()
}
