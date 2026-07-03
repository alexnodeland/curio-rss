//! `curio` — the CLI head over [`curio_core`].
//!
//! Phase 0 stub. The v1 surface (`add / fetch / query / search / export /
//! events tail / opml`, plus `curio export --all`) lands in Phase 3 per
//! `docs/design/roadmap.md`, as thin calls into [`curio_core::CoreHandle`].

use curio_core::CoreHandle;

fn main() {
    let version = CoreHandle::version();
    println!("curio {version} — pre-1.0 workspace reset; see docs/design/roadmap.md");
}
