//! Desktop entry point. Everything lives in the library crate so the
//! bindings-export test and command unit tests can reach it.

// Keep Windows release builds from opening a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    curio_desktop::run();
}
