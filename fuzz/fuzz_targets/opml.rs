//! OPML text → `import_opml`, then export → import again: the second import
//! must accept what the exporter wrote and yield the same subscriptions.

#![no_main]

use curio_core::feeds::{export_opml, import_opml};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(feeds) = import_opml(text) else {
        return;
    };
    let Ok(document) = export_opml("fuzz", &feeds) else {
        panic!("export failed on imported feeds");
    };
    let Ok(again) = import_opml(&document) else {
        panic!("our own export did not re-import:\n{document}");
    };
    assert_eq!(again.len(), feeds.len(), "export -> import lost feeds");
});
