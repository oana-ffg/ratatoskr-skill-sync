fn main() {
    // Cargo tests/checks don't bundle a sidecar. Building it recursively here would deadlock
    // Cargo's workspace lock. Tauri CLI packaging runs prepare:helper before this script.
    let target = std::env::var("TARGET").expect("Cargo TARGET missing");
    let extension = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let helpers: Vec<_> = vec!["ratatoskr"]
        .into_iter()
        .map(|name| format!("binaries/{name}-{target}{extension}"))
        .collect();
    for helper in &helpers {
        println!("cargo:rerun-if-changed={helper}");
    }
    if helpers
        .iter()
        .any(|helper| !std::path::Path::new(helper).is_file())
    {
        if std::env::var_os("TAURI_ENV_TARGET_TRIPLE").is_some() {
            panic!(
                "Bundled helper missing. From desktop/, run: npm run prepare:helper (or npm run prepare:helper -- --debug for development)"
            );
        }
        let mut config: serde_json::Value = std::env::var("TAURI_CONFIG")
            .map(|s| serde_json::from_str(&s).expect("Invalid TAURI_CONFIG"))
            .unwrap_or_else(|_| serde_json::json!({}));
        config["bundle"]["externalBin"] = serde_json::Value::Null;
        let config = config.to_string();
        // A build script is single-threaded; no other threads read the process environment.
        unsafe {
            std::env::set_var("TAURI_CONFIG", &config);
        }
        println!("cargo:rustc-env=TAURI_CONFIG={config}");
        println!(
            "cargo:warning=Non-bundling build: bundled CLI unavailable. To include it, run npm run prepare:helper -- --debug from desktop/."
        );
    }
    tauri_build::build();
}
