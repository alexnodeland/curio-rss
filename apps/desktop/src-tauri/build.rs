//! Build script for the desktop head.
//!
//! `tauri_build::build()` validates `tauri.conf.json` + `capabilities/`
//! and generates the context embedded by `tauri::generate_context!`.

fn main() {
    // The codegen embeds `frontendDist` (../build) at compile time and
    // refuses a missing path. The pure-Rust gates (clippy/test/coverage,
    // locally and in CI) must build this crate with no npm step, so make
    // sure the directory exists; the real assets are produced by
    // `npm run build` before `tauri build` packages anything.
    if let Err(error) = std::fs::create_dir_all("../build") {
        panic!("could not create the frontendDist placeholder ../build: {error}");
    }
    tauri_build::build();
}
