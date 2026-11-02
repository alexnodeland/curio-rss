//! The nasty-feed corpus (issue #133): hundreds of hostile, malformed and
//! merely weird feed inputs run through the real ingest path — `parse_feed`
//! → the content pipeline → markdown — and every one must end in a
//! *classified* outcome (parsed with N entries, or rejected as not-a-feed)
//! with no panic and with stored content that passes the inertness audit.
//!
//! The corpus is mostly generated, deterministically, at test time — so the
//! committed fixtures stay a few KB (the blob guard caps any file at 1 MB):
//!
//! - **seeds**: every committed file under `fixtures/feeds/` and
//!   `fixtures/nasty/` (each with a pinned expected outcome below);
//! - **byte-level variants**: BOMs, UTF-16, Latin-1 declared and undeclared,
//!   control characters, empty and whitespace-only bodies;
//! - **structural extremes**: deep nesting, a huge entry, thousands of
//!   entries, a huge title, attribute and namespace floods;
//! - **mutants**: a seeded splitmix64 mutator stacks truncations, byte
//!   flips, span deletes/duplications and hostile-token splices over every
//!   seed. Same seed, same corpus, every run, on every OS.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
#[path = "common/inert.rs"]
mod inert;

use std::panic::{AssertUnwindSafe, catch_unwind};

use curio_core::content::{first_image, process, to_markdown};
use curio_core::feeds::{ParsedFeed, parse_feed};

/// The fetch URL every corpus input is parsed against.
const BASE: &str = "https://corpus.example.com/feed.xml";

/// Mutants derived from each seed.
const MUTANTS_PER_SEED: u64 = 24;

/// The acceptance floor: the corpus test iterates at least this many inputs.
const MIN_INPUTS: usize = 500;

/// Nesting depth of the deep-HTML extremes (see `structural_variants`):
/// far past the ~1000 levels that used to overflow a 2 MB stack in
/// `to_markdown` and abort the process (#244).
const DEEP_HTML_LEVELS: usize = 10_000;

/// What an input is pinned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Parses, yielding exactly this many entries.
    Parses(usize),
    /// Rejected by the parser as not-a-feed.
    Rejects,
}

/// Every committed seed, with its pinned outcome. A new fixture file must be
/// listed here (`committed_seeds_are_all_pinned` fails otherwise), so adding
/// to the corpus always says what the parser is expected to make of it.
const SEEDS: &[(&str, Expect)] = &[
    ("feeds/atom.xml", Expect::Parses(2)),
    ("feeds/feed.json", Expect::Parses(2)),
    ("feeds/reddit_gallery.json", Expect::Rejects),
    ("feeds/reddit_post.json", Expect::Rejects),
    ("feeds/rss2.xml", Expect::Parses(3)),
    ("feeds/youtube-atom.xml", Expect::Parses(2)),
    ("nasty/atom-sloppy.atom", Expect::Parses(3)),
    ("nasty/billion-laughs.xml", Expect::Parses(1)),
    ("nasty/custom-scheme-links.xml", Expect::Parses(3)),
    ("nasty/deep-nesting.xml", Expect::Parses(1)),
    ("nasty/duplicate-guids.xml", Expect::Parses(3)),
    ("nasty/empty-channel.xml", Expect::Parses(0)),
    ("nasty/html-served-as-feed.html", Expect::Rejects),
    ("nasty/jsonfeed-1.0.json", Expect::Parses(2)),
    ("nasty/jsonfeed-1.1-odd.json", Expect::Parses(2)),
    ("nasty/malformed-unclosed.xml", Expect::Rejects),
    ("nasty/media-itunes.xml", Expect::Parses(3)),
    ("nasty/odd-dates.xml", Expect::Parses(13)),
    ("nasty/reddit_hostile.json", Expect::Rejects),
    ("nasty/rss1-rdf.rdf", Expect::Parses(2)),
    ("nasty/truncated.atom", Expect::Rejects),
    ("nasty/xml-base.atom", Expect::Parses(2)),
    ("nasty/xss-feed.xml", Expect::Parses(2)),
];

/// The classified outcome of one input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// A feed with this many entries, every one ingested cleanly.
    Parsed(usize),
    /// Not a feed (the parser said so — an error, not a panic).
    Rejected,
}

/// One corpus input.
struct Input {
    label: String,
    bytes: Vec<u8>,
}

fn fixture(rel: &str) -> Vec<u8> {
    std::fs::read(common::manifest_dir().join("../../fixtures").join(rel))
        .unwrap_or_else(|err| panic!("read fixtures/{rel}: {err}"))
}

/// The real ingest path for one input, minus storage: parse, then every
/// entry through sanitize → extract → text → markdown, auditing what would
/// be stored and exported. `Err` is an audit violation.
fn ingest(bytes: &[u8]) -> Result<Outcome, String> {
    let Ok(feed) = parse_feed(bytes, Some(BASE)) else {
        return Ok(Outcome::Rejected);
    };
    audit_feed(&feed)?;
    Ok(Outcome::Parsed(feed.entries.len()))
}

/// Audits what one parsed feed would store. `entry.link` and
/// `feed.meta.site_url` carry no policy yet. Both are stored unscreened
/// (`javascript:` and custom schemes included; see `xss-feed.xml` and
/// `custom-scheme-links.xml`), and #262 tracks screening them and asserting
/// the result here. What this audit does cover is the dangerous half of the
/// entry link: its use as the content base.
fn audit_feed(feed: &ParsedFeed) -> Result<(), String> {
    for entry in &feed.entries {
        if entry.dedupe_key.is_empty() {
            return Err(format!("entry {:?} has an empty dedupe key", entry.title));
        }
        if entry.title.trim().is_empty() {
            return Err("an entry title is blank (expected the `(untitled)` fallback)".into());
        }
        let base = entry.link.as_deref().unwrap_or(BASE);
        let processed = process(&entry.content_html, Some(base));
        inert::audit_html(&processed.html)?;
        if let Some(image) = first_image(&processed.html)
            && !(image.starts_with("http://") || image.starts_with("https://"))
        {
            return Err(format!("non-http(s) inline lead image {image:?}"));
        }
        if let Some(image) = &entry.lead_image
            && !(image.starts_with("http://") || image.starts_with("https://"))
        {
            return Err(format!("non-http(s) feed lead image {image:?}"));
        }
        let markdown = to_markdown(&processed.html).map_err(|err| format!("markdown: {err}"))?;
        inert::audit_markdown(&markdown)?;
    }
    Ok(())
}

/// Runs one input with panics caught, so a single bad input names itself
/// instead of aborting the corpus.
fn classify(input: &Input) -> Result<Outcome, String> {
    match catch_unwind(AssertUnwindSafe(|| ingest(&input.bytes))) {
        Ok(Ok(outcome)) => Ok(outcome),
        Ok(Err(violation)) => Err(format!("{}: audit violation: {violation}", input.label)),
        Err(panic) => {
            let message = panic
                .downcast_ref::<&str>()
                .map(ToString::to_string)
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_owned());
            Err(format!("{}: PANIC: {message}", input.label))
        }
    }
}

// ── the deterministic generators ──────────────────────────────────────────

/// splitmix64: tiny, seedable, identical on every platform.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform-enough in `0..bound` (`bound > 0`).
    fn below(&mut self, bound: usize) -> usize {
        // usize is at most 64 bits on every supported target.
        #[allow(clippy::cast_possible_truncation)]
        let pick = (self.next() % bound as u64) as usize;
        pick
    }
}

/// FNV-1a over the label: each seed gets its own mutant stream.
fn label_seed(label: &str) -> u64 {
    label.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Fragments real feeds get wrong, plus the attacks the sanitizer exists for.
const HOSTILE_TOKENS: &[&[u8]] = &[
    b"<",
    b">",
    b"&",
    b"&amp;",
    b"&#0;",
    b"&#xD800;",
    b"&#x10FFFF;",
    b"&undefined;",
    b"]]>",
    b"<![CDATA[",
    b"<!--",
    b"-->",
    b"\0",
    b"\xff\xfe",
    b"\xc3",
    b"\xe2\x80\xae",
    "🎌👍🏽".as_bytes(),
    b"<script>alert(1)</script>",
    b"\"><img src=x onerror=alert(1)>",
    b"javascript:alert(1)",
    b"<a href=\"javascript:alert(1)\">x</a>",
    b"&lt;script&gt;alert(1)&lt;/script&gt;",
    b"<item>",
    b"</item>",
    b"<entry>",
    b"</entry>",
    b"</channel>",
    b"<guid></guid>",
    b"<link>javascript:alert(1)</link>",
    // Parseable non-http(s) links: the entry link is the content base, so
    // these reach the base-resolution path (cannot-be-a-base links like
    // `javascript:` never do).
    b"<link>foo://x/</link>",
    b"<link>httpsupdated://xss.example.com/one</link>",
    b"<link href=\"foo://x/dir/\"/>",
    b"\"url\": \"foo://x/\",",
    b"<pubDate>not a date</pubDate>",
    b" xmlns:a=\"urn:x\"",
    b" xml:base=\"javascript:alert(1)//\"",
    b" xml:base=\"foo://x/dir/\"",
    b"<!DOCTYPE x [<!ENTITY a \"b\">]>",
    b"<?xml version=\"1.0\" encoding=\"UTF-16\"?>",
    b"{",
    b"}",
    b"[",
    b"\"",
    b",",
    b"null",
    b"\"items\": [{}]",
    b"\\u0000",
    b"\\ud800",
];

/// Payloads that keep a document well-formed when spliced into element
/// text, so the mutant still *parses* and its content reaches the sanitizer.
const TEXT_TOKENS: &[&str] = &[
    "&amp;",
    "&lt;script&gt;alert(1)&lt;/script&gt;",
    "&lt;img src=x onerror=alert(1)&gt;",
    "&lt;a href=\"javascript:alert(1)\"&gt;x&lt;/a&gt;",
    "&lt;svg onload=alert(1)&gt;&lt;/svg&gt;",
    "&lt;iframe src=\"https://evil.example/\"&gt;&lt;/iframe&gt;",
    "&lt;a href=\"data:text/html,x\"&gt;d&lt;/a&gt;",
    "&lt;p style=\"x:expression(alert(1))\"&gt;s&lt;/p&gt;",
    "<![CDATA[<script>alert(1)</script><img src=x onerror=alert(1)>]]>",
    "<![CDATA[<a href=\" javascript:alert(1)\">j</a><base href=\"https://evil.example/\">]]>",
    "&#x10FFFF;",
    "\u{202e}reversed",
    "🎌👍🏽 日本語",
    "javascript:alert(1)",
    // Spliced at the start of a `<link>`, these turn it into a parseable
    // custom-scheme base; the relative URLs give that base work to do.
    "foo://x/",
    "httpsupdated:",
    "&lt;img src=\"rel.png\"&gt;&lt;a href=\"rel/ok\"&gt;r&lt;/a&gt;&lt;a href=\"//cdn.example/p\"&gt;p&lt;/a&gt;",
    "<![CDATA[<img src=\"x\"><a href=\"relative/ok\">r</a>]]>",
    "averyveryveryveryveryveryveryveryveryveryveryveryveryverylongword",
];

/// One seeded mutant of `seed`.
///
/// `text_safe` mutants splice [`TEXT_TOKENS`] into element text (just
/// after a `>`), so most still parse and carry hostile content into the
/// pipeline; the others stack one to four arbitrary byte-level mutations
/// (truncation, byte flips, span deletes/duplications, [`HOSTILE_TOKENS`]
/// splices anywhere) and mostly exercise the parser's rejection paths.
fn mutate(seed: &[u8], rng: &mut Rng, text_safe: bool) -> Vec<u8> {
    let mut out = seed.to_vec();
    if text_safe {
        for _ in 0..=rng.below(3) {
            let anchors: Vec<usize> = out
                .iter()
                .enumerate()
                .filter(|&(_, &byte)| byte == b'>')
                .map(|(index, _)| index + 1)
                .collect();
            let at = if anchors.is_empty() {
                rng.below(out.len() + 1)
            } else {
                anchors[rng.below(anchors.len())]
            };
            let token = TEXT_TOKENS[rng.below(TEXT_TOKENS.len())];
            out.splice(at..at, token.bytes());
        }
        return out;
    }
    for _ in 0..=rng.below(4) {
        let len = out.len().max(1);
        match rng.below(6) {
            0 => out.truncate(rng.below(len)),
            1 => {
                if !out.is_empty() {
                    let at = rng.below(out.len());
                    out[at] = rng.next().to_le_bytes()[0];
                }
            }
            2 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(64)).min(out.len());
                if start < end {
                    out.drain(start..end);
                }
            }
            3 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(256)).min(out.len());
                if start < end {
                    let span = out[start..end].to_vec();
                    let at = rng.below(out.len() + 1);
                    out.splice(at..at, span.iter().copied().cycle().take(span.len() * 4));
                }
            }
            _ => {
                let token = HOSTILE_TOKENS[rng.below(HOSTILE_TOKENS.len())];
                let at = rng.below(out.len() + 1);
                out.splice(at..at, token.iter().copied());
            }
        }
    }
    out
}

fn utf16(text: &str, little_endian: bool) -> Vec<u8> {
    let mut out = if little_endian {
        vec![0xff, 0xfe]
    } else {
        vec![0xfe, 0xff]
    };
    for unit in text.encode_utf16() {
        out.extend(if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        });
    }
    out
}

fn rss_with_items(items: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>Generated</title><link>https://gen.example.com/</link><description>d</description>{items}</channel></rss>"#
    )
}

/// Encoding tricks real servers ship, built byte by byte.
fn byte_variants() -> Vec<Input> {
    let rss2 = String::from_utf8(fixture("feeds/rss2.xml")).unwrap();
    let one_item = |title: &[u8], decl: &[u8]| {
        let mut out = decl.to_vec();
        out.extend_from_slice(
            b"<rss version=\"2.0\"><channel><title>Encodings</title><link>https://enc.example.com/</link><description>d</description><item><guid>e-1</guid><title>",
        );
        out.extend_from_slice(title);
        out.extend_from_slice(b"</title></item></channel></rss>");
        out
    };
    let mut bom_utf8 = vec![0xef, 0xbb, 0xbf];
    bom_utf8.extend_from_slice(rss2.as_bytes());
    let mut whitespace_lead = b"\n\n   \t".to_vec();
    whitespace_lead.extend_from_slice(rss2.as_bytes());
    vec![
        Input {
            label: "bytes/bom-utf8".into(),
            bytes: bom_utf8,
        },
        Input {
            label: "bytes/utf16le-bom".into(),
            bytes: utf16(&rss2, true),
        },
        Input {
            label: "bytes/utf16be-bom".into(),
            bytes: utf16(&rss2, false),
        },
        Input {
            label: "bytes/utf16le-declared-utf16".into(),
            bytes: utf16(&rss2.replace("UTF-8", "UTF-16"), true),
        },
        Input {
            label: "bytes/latin1-declared".into(),
            bytes: one_item(
                b"Caf\xe9 cr\xe8me",
                b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>",
            ),
        },
        Input {
            label: "bytes/windows1252-declared".into(),
            bytes: one_item(
                b"\x93smart quotes\x94 \x80",
                b"<?xml version=\"1.0\" encoding=\"windows-1252\"?>",
            ),
        },
        Input {
            label: "bytes/latin1-claims-utf8".into(),
            bytes: one_item(
                b"Caf\xe9 \xff\xfe",
                b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            ),
        },
        Input {
            label: "bytes/unknown-encoding".into(),
            bytes: one_item(
                b"plain",
                b"<?xml version=\"1.0\" encoding=\"x-klingon-9\"?>",
            ),
        },
        Input {
            label: "bytes/control-chars".into(),
            bytes: one_item(
                b"NUL\x00 bell\x07 esc\x1b ff\x0c",
                b"<?xml version=\"1.0\"?>",
            ),
        },
        Input {
            label: "bytes/overlong-utf8".into(),
            bytes: one_item(b"\xc0\xbcscript\xc0\xbe \xed\xa0\x80", b""),
        },
        Input {
            label: "bytes/whitespace-before-decl".into(),
            bytes: whitespace_lead,
        },
        Input {
            label: "bytes/empty".into(),
            bytes: Vec::new(),
        },
        Input {
            label: "bytes/whitespace-only".into(),
            bytes: b" \n\t\r\n ".to_vec(),
        },
        Input {
            label: "bytes/garbage".into(),
            bytes: b"just text, not a feed \x00\xff".to_vec(),
        },
        Input {
            label: "bytes/json-null".into(),
            bytes: b"null".to_vec(),
        },
        Input {
            label: "bytes/json-empty-object".into(),
            bytes: b"{}".to_vec(),
        },
        Input {
            label: "bytes/jsonfeed-wrong-types".into(),
            bytes: br#"{"version":"https://jsonfeed.org/version/1.1","title":7,"items":"nope"}"#
                .to_vec(),
        },
    ]
}

/// Size and shape extremes, generated so nothing large is committed.
fn structural_variants() -> Vec<Input> {
    // feed-rs walks XML iteratively, so deep *XML* is safe far past this
    // (to 65_535 levels, where quick-xml's u16 namespace-depth counter
    // overflows — issue #246).
    let deep_xml = format!("{}{}", "<x>".repeat(5_000), "</x>".repeat(5_000));
    // Deep *HTML*: htmd's walk recurses per level, so `to_markdown` caps
    // the nesting it hands htmd (#244). Divs are the slow shape to parse
    // (html5ever scans the open elements for every start tag); table
    // cells are scope boundaries, so the tables reach 40 000 levels fast.
    let deep_html_body = format!(
        "{}deep{}",
        "<div>".repeat(DEEP_HTML_LEVELS),
        "</div>".repeat(DEEP_HTML_LEVELS)
    );
    let deep_tables_body = format!(
        "{}deep{}",
        "<table><tr><td>".repeat(DEEP_HTML_LEVELS),
        "</td></tr></table>".repeat(DEEP_HTML_LEVELS)
    );
    let huge_entry = format!(
        "<item><guid>huge</guid><title>Huge</title><description><![CDATA[{}]]></description></item>",
        "<p>A paragraph of an enormous entry, with <b>markup</b> &amp; links <a href=\"https://x.example/\">x</a>.</p>"
            .repeat(6_000)
    );
    let many_items: String = (0..2_000)
        .map(|i| format!("<item><guid>g{i}</guid><title>Item {i}</title><description>&lt;p&gt;{i}&lt;/p&gt;</description></item>"))
        .collect::<Vec<_>>()
        .concat();
    let many_attrs = (0..5_000)
        .map(|i| format!(" a{i}=\"{i}\""))
        .collect::<Vec<_>>()
        .concat();
    let many_namespaces: String = (0..2_000)
        .map(|i| format!(" xmlns:n{i}=\"urn:n:{i}\""))
        .collect::<Vec<_>>()
        .concat();
    let json_items: String = (0..1_000)
        .map(|i| format!(r#"{{"id":"j{i}","content_html":"<p>{i}</p>"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    vec![
        Input {
            label: "shape/deep-unknown-elements".into(),
            bytes: rss_with_items(&format!("<item><guid>deep</guid>{deep_xml}</item>")).into_bytes(),
        },
        Input {
            label: "shape/deep-html-content".into(),
            bytes: rss_with_items(&format!(
                "<item><guid>deep-html</guid><description><![CDATA[{deep_html_body}]]></description></item>"
            ))
            .into_bytes(),
        },
        Input {
            label: "shape/deep-html-tables".into(),
            bytes: rss_with_items(&format!(
                "<item><guid>deep-tables</guid><description><![CDATA[{deep_tables_body}]]></description></item>"
            ))
            .into_bytes(),
        },
        Input { label: "shape/huge-entry".into(), bytes: rss_with_items(&huge_entry).into_bytes() },
        Input { label: "shape/many-items".into(), bytes: rss_with_items(&many_items).into_bytes() },
        Input {
            label: "shape/huge-title".into(),
            bytes: rss_with_items(&format!(
                "<item><guid>t</guid><title>{}</title></item>",
                "long title ".repeat(20_000)
            ))
            .into_bytes(),
        },
        Input {
            label: "shape/attribute-flood".into(),
            bytes: rss_with_items(&format!("<item{many_attrs}><guid>attrs</guid></item>")).into_bytes(),
        },
        Input {
            label: "shape/namespace-flood".into(),
            bytes: format!(
                r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"{many_namespaces}><title>ns</title><entry><id>ns-1</id><title>t</title></entry></feed>"#
            )
            .into_bytes(),
        },
        Input {
            label: "shape/jsonfeed-many-items".into(),
            bytes: format!(r#"{{"version":"https://jsonfeed.org/version/1.1","title":"many","items":[{json_items}]}}"#)
                .into_bytes(),
        },
        Input {
            label: "shape/jsonfeed-deep-nesting".into(),
            bytes: format!(
                r#"{{"version":"https://jsonfeed.org/version/1.1","title":"deep","x":{}1{},"items":[]}}"#,
                "[".repeat(100),
                "]".repeat(100)
            )
            .into_bytes(),
        },
    ]
}

fn seeds() -> Vec<Input> {
    SEEDS
        .iter()
        .map(|(rel, _)| Input {
            label: (*rel).to_owned(),
            bytes: fixture(rel),
        })
        .collect()
}

/// The whole corpus: seeds, variants, extremes, then the mutants of every
/// seed and byte variant.
fn corpus() -> Vec<Input> {
    let mut inputs = seeds();
    inputs.extend(byte_variants());
    inputs.extend(structural_variants());
    let parents: Vec<(String, Vec<u8>)> = inputs
        .iter()
        // Mutating the multi-hundred-KB extremes adds runtime, not coverage.
        .filter(|input| input.bytes.len() < 64 * 1024)
        .map(|input| (input.label.clone(), input.bytes.clone()))
        .collect();
    for (label, bytes) in parents {
        let mut rng = Rng::new(label_seed(&label));
        for n in 0..MUTANTS_PER_SEED {
            inputs.push(Input {
                label: format!("{label}#mutant-{n}"),
                bytes: mutate(&bytes, &mut rng, n % 2 == 0),
            });
        }
    }
    inputs
}

// ── the gate ───────────────────────────────────────────────────────────────

/// The acceptance test: every input ends in a classified outcome — no
/// panic, no audit violation — and the corpus is at least `MIN_INPUTS`.
#[test]
fn the_nasty_corpus_parses_without_panic_and_stores_inert_content() {
    let corpus = corpus();
    assert!(
        corpus.len() >= MIN_INPUTS,
        "the corpus shrank to {} inputs (floor {MIN_INPUTS})",
        corpus.len()
    );

    let mut failures = Vec::new();
    let (mut parsed, mut rejected) = (0usize, 0usize);
    for input in &corpus {
        match classify(input) {
            Ok(Outcome::Parsed(_)) => parsed += 1,
            Ok(Outcome::Rejected) => rejected += 1,
            Err(failure) => failures.push(failure),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus inputs failed:\n{}",
        failures.len(),
        corpus.len(),
        failures.join("\n")
    );
    eprintln!(
        "nasty corpus: {} inputs — {parsed} parsed, {rejected} rejected, 0 panics",
        corpus.len()
    );
    // A corpus where everything parses (or nothing does) is not exercising
    // both halves of the classification.
    assert!(parsed >= 250, "only {parsed} inputs parsed");
    assert!(rejected >= 250, "only {rejected} inputs were rejected");
}

/// Each committed seed lands in exactly its pinned class.
#[test]
fn committed_seeds_classify_as_pinned() {
    for (rel, expected) in SEEDS {
        let input = Input {
            label: (*rel).to_owned(),
            bytes: fixture(rel),
        };
        let outcome = classify(&input).unwrap();
        let actual = match outcome {
            Outcome::Parsed(n) => Expect::Parses(n),
            Outcome::Rejected => Expect::Rejects,
        };
        assert_eq!(actual, *expected, "{rel}");
    }
}

/// No committed corpus file goes unpinned.
#[test]
fn committed_seeds_are_all_pinned() {
    let root = common::manifest_dir().join("../../fixtures");
    for dir in ["feeds", "nasty"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let rel = format!("{dir}/{name}");
            assert!(
                SEEDS.iter().any(|(seed, _)| *seed == rel),
                "fixtures/{rel} is not pinned in SEEDS"
            );
        }
    }
}

/// The generator is deterministic: two builds are byte-identical.
#[test]
fn the_corpus_is_deterministic() {
    let digest = |inputs: &[Input]| -> Vec<(String, u64)> {
        inputs
            .iter()
            .map(|input| {
                (
                    input.label.clone(),
                    label_seed(&String::from_utf8_lossy(&input.bytes)),
                )
            })
            .collect()
    };
    assert_eq!(digest(&corpus()), digest(&corpus()));
}

// ── what the parser makes of each nasty shape ─────────────────────────────

fn parse(rel: &str) -> ParsedFeed {
    parse_feed(&fixture(rel), Some(BASE)).unwrap()
}

#[test]
fn rss1_rdf_items_parse_with_dc_metadata() {
    let feed = parse("nasty/rss1-rdf.rdf");
    assert_eq!(feed.meta.title.as_deref(), Some("RDF Site Summary"));
    let one = &feed.entries[0];
    assert_eq!(one.title, "RDF item one");
    assert_eq!(one.link.as_deref(), Some("https://rdf.example.org/one"));
    assert!(one.published.is_some(), "dc:date is the RDF publish date");
}

#[test]
fn json_feed_1_0_and_1_1_both_parse() {
    let v10 = parse("nasty/jsonfeed-1.0.json");
    assert_eq!(v10.entries[0].title, "Version one item");
    assert!(v10.entries[0].published.is_some());
    assert_eq!(v10.entries[1].title, "(untitled)");
    assert!(
        v10.entries[1].published.is_none(),
        "unparsable date drops to None"
    );

    let v11 = parse("nasty/jsonfeed-1.1-odd.json");
    let blank_id = &v11.entries[0];
    assert!(blank_id.guid.is_none(), "a whitespace id is no id");
    assert!(!blank_id.dedupe_key.starts_with("guid:"));
    let processed = process(&blank_id.content_html, Some(BASE));
    inert::audit_html(&processed.html).unwrap();
    assert!(
        processed.text.contains("クリック"),
        "the text around the attack survives"
    );
    let titles: Vec<&str> = v11.entries.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(
        titles,
        ["空白の id", "Dup A"],
        "a repeated id with the same (absent) link is an exact repeat: the first copy wins"
    );
}

#[test]
fn duplicate_guids_get_unique_keys_and_exact_repeats_drop() {
    let feed = parse("nasty/duplicate-guids.xml");
    let keys: Vec<&str> = feed.entries.iter().map(|e| e.dedupe_key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "guid+link:same-guid\u{1f}https://dup.example.com/a",
            "guid+link:same-guid\u{1f}https://dup.example.com/b",
            "link:https://dup.example.com/a",
        ],
        "a guid reused across links (trimmed first) re-keys to guid+link; \
         the second guid-less item on the same link is an exact repeat and drops"
    );
}

#[test]
fn odd_dates_parse_or_drop_without_failing_the_feed() {
    let feed = parse("nasty/odd-dates.xml");
    let published = |guid: &str| {
        feed.entries
            .iter()
            .find(|entry| entry.guid.as_deref() == Some(guid))
            .unwrap_or_else(|| panic!("no entry {guid}"))
            .published
    };
    for absent in ["d-none", "d-empty", "d-garbage"] {
        assert!(published(absent).is_none(), "{absent} has no usable date");
    }
    for present in ["d-zone", "d-num", "d-iso", "d-dc", "d-epoch"] {
        assert!(
            published(present).is_some(),
            "{present} carries a real date"
        );
    }
}

#[test]
fn media_and_itunes_namespaces_yield_http_lead_images_only() {
    let feed = parse("nasty/media-itunes.xml");
    assert_eq!(
        feed.entries[0].lead_image.as_deref(),
        Some("https://pod.example.com/ep1-thumb.jpg"),
        "media:thumbnail wins"
    );
    assert_eq!(
        feed.entries[1].lead_image.as_deref(),
        Some("https://pod.example.com/ep2.png"),
        "an image enclosure is the fallback"
    );
    let text_only = &feed.entries[2];
    assert!(
        text_only.lead_image.is_none(),
        "a javascript: thumbnail is dropped"
    );
    let processed = process(&text_only.content_html, Some(BASE));
    inert::audit_html(&processed.html).unwrap();
    assert!(
        processed.text.contains("<script>alert(1)</script>"),
        "media:description is text: the markup survives as inert characters"
    );
}

#[test]
fn xml_base_resolves_relative_links() {
    let feed = parse("nasty/xml-base.atom");
    assert_eq!(
        feed.entries[0].link.as_deref(),
        Some("https://base.example.org/blog/posts/one")
    );
    assert_eq!(
        feed.entries[1].link.as_deref(),
        Some("https://other.example.org/root/two"),
        "an entry-level xml:base overrides the feed's"
    );
}

/// The audit has teeth: raw (unsanitized) payloads fail it, and the
/// shapes htmd legitimately emits for inert text pass.
#[test]
fn the_inertness_audit_flags_live_payloads_only() {
    for live in [
        "<script>alert(1)</script>",
        "<img src=x onerror=alert(1)>",
        "<a href=\"javascript:alert(1)\">x</a>",
        "<p style=\"color:red\">x</p>",
        "<svg></svg>",
        // Disguises a browser still reads as the scheme they hide.
        "<a href=\"\u{1}javascript:alert(1)\">x</a>",
        "<a href=\"java\tscript:alert(1)\">x</a>",
        "<img src=\" DATA:image/png,x\">",
        // Stored URLs are absolute: a relative one resolves against the
        // reader, not the source.
        "<a href=\"relative/ok\">r</a>",
        "<a href=\"#not-prefixed\">f</a>",
    ] {
        assert!(
            inert::audit_html(live).is_err(),
            "html audit passed {live:?}"
        );
    }
    for stored in [
        "<p><a href=\"https://x.example/\">ok</a></p>",
        // The `content_pipeline` fuzz find: WHATWG parsing strips the C0
        // control, so this is an https image to the webview (and to
        // ammonia, which let it through).
        "<img src=\"\u{2}https://example.com/cat.jpg&quot;\">",
        "<a href=\"HTTPS://x.example/\">upper</a>",
        "<a href=\"#user-content-fn1\">1</a>",
    ] {
        assert!(
            inert::audit_html(stored).is_ok(),
            "html audit flagged {stored:?}"
        );
    }

    for live in [
        "[x](javascript:alert(1))",
        "![i]( <data:image/svg+xml,x>)",
        "text <script>alert(1)</script>",
        "<img src=x onerror=alert(1)>",
        "<a href=\"javascript:alert(1)\">x</a>",
        "a lone ` backtick hides nothing <script>",
        "[x]( java\tscript:alert(1))",
        "[x](\u{1}javascript:alert(1))",
        // An escaped backtick opens no span, so the tag between it and the
        // next backtick is live.
        "\\`<script>alert(1)</script>`",
        // A run closes only on a run of the same length.
        "a ``` <script>alert(1)</script> `",
        // Four spaces is an indented code line, not a fence: the lines
        // after it are prose.
        "    ```\n<script>alert(1)</script>",
        // A fence closes only on the same marker.
        "~~~\nx\n```\n~~~\n<script>alert(1)</script>",
    ] {
        assert!(
            inert::audit_markdown(live).is_err(),
            "markdown audit passed {live:?}"
        );
    }
    for inert_text in [
        "Show notes \\<script>alert(1)\\</script> as text.",
        "`<script>` in a code span",
        "```html\n<script>alert(1)</script>\n```",
        "a < b and <3",
        "[ok](https://x.example/)",
        "``a ` <script>alert(1)</script> ``",
        "``a`<script>alert(1)</script>`",
        "   ```\n<script>alert(1)</script>\n```",
        "~~~~\n<script>alert(1)</script>\n~~~\n~~~~",
    ] {
        assert!(
            inert::audit_markdown(inert_text).is_ok(),
            "markdown audit flagged {inert_text:?}"
        );
    }
}

#[test]
fn html_served_as_a_feed_is_rejected_not_parsed() {
    assert!(parse_feed(&fixture("nasty/html-served-as-feed.html"), Some(BASE)).is_err());
}

#[test]
fn the_xss_feed_stores_inert_content_and_keeps_the_story() {
    let feed = parse("nasty/xss-feed.xml");
    for entry in &feed.entries {
        let processed = process(&entry.content_html, entry.link.as_deref().or(Some(BASE)));
        inert::audit_html(&processed.html).unwrap();
        let markdown = to_markdown(&processed.html).unwrap();
        inert::audit_markdown(&markdown).unwrap();
        assert!(
            !markdown.to_lowercase().contains("evil.example"),
            "an attacker URL survived:\n{markdown}"
        );
    }
    let first = process(
        &feed.entries[0].content_html,
        Some("https://xss.example.com/one"),
    );
    assert!(first.html.contains("<strong>content</strong>"));
    assert!(first.html.contains("https://xss.example.com/relative/ok"));
}

/// The first `feed_parse` fuzz find, pinned: an entry link with a
/// parseable custom scheme is the content base, and nothing relative may
/// resolve onto it.
#[test]
fn a_custom_scheme_entry_link_plants_no_link_in_stored_content() {
    let feed = parse("nasty/custom-scheme-links.xml");
    for entry in &feed.entries {
        let base = entry.link.as_deref().unwrap_or(BASE);
        let processed = process(&entry.content_html, Some(base));
        inert::audit_html(&processed.html).unwrap();
        let scheme = base.split(':').next().unwrap();
        for leaked in [format!("{scheme}:"), "cdn.example".to_owned()] {
            assert!(
                !processed.html.contains(&leaked),
                "{base}: {leaked:?} reached stored content:\n{}",
                processed.html
            );
        }
        assert!(
            !processed.html.contains("src="),
            "{base}: a relative src survived without an http(s) base:\n{}",
            processed.html
        );
    }
    let first = process(
        &feed.entries[0].content_html,
        feed.entries[0].link.as_deref(),
    );
    assert!(
        first.html.contains("<strong>content</strong>"),
        "{}",
        first.html
    );
    assert!(
        first.html.contains("href=\"https://ok.example/absolute\""),
        "absolute http(s) links are unaffected:\n{}",
        first.html
    );
    assert!(!first.html.contains("relative/ok"), "{}", first.html);
}

#[test]
fn deep_html_exports_with_its_text() {
    // The gate proves no depth aborts the export; this pins that the text
    // under 40 000 levels of nesting reaches the markdown too (#244). The
    // tables extreme parses fast; the divs' text is pinned in
    // `content::markdown`'s unit tests and the `props` deep-nesting suite.
    let input = structural_variants()
        .into_iter()
        .find(|input| input.label == "shape/deep-html-tables")
        .unwrap();
    let feed = parse_feed(&input.bytes, Some(BASE)).unwrap();
    let processed = process(&feed.entries[0].content_html, Some(BASE));
    assert_eq!(processed.text, "deep");
    let markdown = to_markdown(&processed.html).unwrap();
    assert!(markdown.contains("deep"), "{markdown}");
}

#[test]
fn the_deep_nesting_seed_exports_every_word() {
    // The committed regression seed: 900 levels of mixed wrappers (lists,
    // quotes, sections), past the depth that aborted an uncapped export.
    let feed = parse("nasty/deep-nesting.xml");
    let processed = process(
        &feed.entries[0].content_html,
        feed.entries[0].link.as_deref(),
    );
    let markdown = to_markdown(&processed.html).unwrap();
    inert::audit_markdown(&markdown).unwrap();
    let words: Vec<&str> = processed.text.split_whitespace().collect();
    assert_eq!(words.len(), 12, "{words:?}");
    for word in words {
        assert!(markdown.contains(word), "lost {word:?}:\n{markdown}");
    }
    assert!(markdown.starts_with("Before the nesting."), "{markdown}");
    assert!(markdown.ends_with("After the nesting."), "{markdown}");
}

#[test]
fn the_entity_bomb_is_not_expanded() {
    let feed = parse("nasty/billion-laughs.xml");
    let title_len = feed.entries[0].title.len();
    assert!(title_len < 1_000, "an entity expanded to {title_len} bytes");
    assert!(
        !feed.entries[0].content_html.contains("root:"),
        "an external entity was resolved"
    );
}
