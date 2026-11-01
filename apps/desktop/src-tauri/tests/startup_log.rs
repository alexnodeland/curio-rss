//! A startup failure is written to the rotating log, the only record a
//! bundled app (no visible stderr) leaves of why it would not open.
//!
//! Its own test binary: `logging::init` installs the process-global
//! tracing subscriber, which no other test may share.

use std::path::Path;

use curio_desktop::startup::{self, StartupFailureClass};

fn read_logs(logs: &Path) -> String {
    let mut text = String::new();
    let entries = std::fs::read_dir(logs).unwrap_or_else(|e| panic!("read logs dir: {e}"));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("log entry: {e}"));
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("curio.log") {
            text.push_str(
                &std::fs::read_to_string(entry.path()).unwrap_or_else(|e| panic!("read log: {e}")),
            );
        }
    }
    text
}

#[test]
fn a_startup_failure_lands_in_curio_log() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
    let profile = dir.path().join("profile");
    std::fs::create_dir_all(&profile).unwrap_or_else(|e| panic!("mkdir: {e}"));
    std::fs::write(
        profile.join("curio.db"),
        b"not a database, but long enough for a header",
    )
    .unwrap_or_else(|e| panic!("write: {e}"));
    let logs = profile.join("logs");
    curio_desktop::logging::init(&logs);

    let Err(failure) = startup::boot(Ok(profile.clone())) else {
        panic!("a garbage database opened");
    };
    assert_eq!(failure.class, StartupFailureClass::Corrupt);

    let log = read_logs(&logs);
    assert!(log.contains("startup failed"), "log: {log}");
    assert!(log.contains("ERROR"), "log: {log}");
    assert!(log.contains("Corrupt"), "log: {log}");
    assert!(log.contains("not a readable Curio database"), "log: {log}");
}
