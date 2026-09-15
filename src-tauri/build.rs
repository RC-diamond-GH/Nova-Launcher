fn main() {
    // `tauri-build` only registers `tauri.conf.json` and `capabilities` with cargo, so
    // regenerating the icons (`npx tauri icon src/assets/logo.png`) would not re-run this
    // script: cargo would keep the previous `resource.lib` and link the stale icon into the
    // executable. Tracking the icon directory makes an icon change rebuild it.
    println!("cargo:rerun-if-changed=icons");

    tauri_build::build()
}
