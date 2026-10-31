//! The cold-start bench's output check (`cargo run -p xtask -- bench-output`).
//!
//! The nightly `bench` job pipes `cargo bench … -- --output-format bencher`
//! into `benchmark-action/github-action-benchmark` (`tool: cargo`), which
//! keeps only the lines shaped exactly like libtest's bencher report:
//!
//! ```text
//! test <name> ... bench:   <value> <unit>/iter (+/- <range>)
//! ```
//!
//! Anything criterion prints between `test <name> ... ` and `bench:` breaks
//! that line in two, and the tracker then finds no result — the job goes red
//! without the regression gate ever running (issue #185: a stale baseline
//! directory made criterion print `Criterion.rs ERROR: …` mid-line). This
//! check runs before the tracker, parses the output with the same rule, and
//! fails with the actual cause: a criterion error line, no parseable result,
//! or a missing expected benchmark.

/// One parsed `test <name> ... bench: <value> <unit> (+/- <range>)` line.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchResult {
    /// Benchmark id, e.g. `cold_start/open_and_first_page_50k`.
    pub name: String,
    /// The measured value, commas stripped (criterion reports a mean).
    pub value: f64,
    /// The unit, e.g. `ns/iter`.
    pub unit: String,
    /// The `+/-` range, commas stripped.
    pub range: String,
}

/// What criterion prefixes its own error reports with (its `error!` macro).
const CRITERION_ERROR: &str = "Criterion.rs ERROR:";

/// Parse one line the way github-action-benchmark's `cargo` extractor does
/// (`^test (.+)\s+\.\.\.\s+bench:\s+([0-9,.]+) (\w+\/\w+) \(\+\/- ([0-9,.]+)\)$`).
/// `None` for any line the tracker would skip.
#[must_use]
pub fn parse_line(line: &str) -> Option<BenchResult> {
    let rest = line.strip_prefix("test ")?;
    // `(.+)` is greedy: the name runs to the LAST `...` (a benchmark id may
    // itself contain one).
    let (name, after) = rest.rsplit_once("...")?;
    // `(.+)\s+` — a non-empty name, then at least one blank before `...`.
    if !name.ends_with(char::is_whitespace) {
        return None;
    }
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let after = after
        .strip_prefix(char::is_whitespace)?
        .trim_start()
        .strip_prefix("bench:")?;
    let after = after.strip_prefix(char::is_whitespace)?.trim_start();

    let (value, after) = after.split_once(' ')?;
    if !is_number(value) {
        return None;
    }
    let (unit, after) = after.split_once(' ')?;
    if !is_unit(unit) {
        return None;
    }
    let range = after.strip_prefix("(+/- ")?.strip_suffix(')')?;
    if !is_number(range) {
        return None;
    }
    Some(BenchResult {
        name: name.to_owned(),
        value: value.replace(',', "").parse().ok()?,
        unit: unit.to_owned(),
        range: range.replace(',', ""),
    })
}

/// `[0-9,.]+`
fn is_number(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b',' || b == b'.')
}

/// `\w+\/\w+` (JavaScript's `\w` is ASCII-only).
fn is_unit(text: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    text.split_once('/')
        .is_some_and(|(left, right)| word(left) && word(right))
}

/// Check a bencher-format bench log: no criterion error line, at least one
/// result the tracker can parse, and every `expected` benchmark among them.
///
/// # Errors
/// One message per problem found, naming the offending line where there is
/// one.
pub fn check(output: &str, expected: &[&str]) -> Result<Vec<BenchResult>, Vec<String>> {
    let mut problems = Vec::new();
    let mut results = Vec::new();
    for (index, line) in output.lines().enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.contains(CRITERION_ERROR) {
            problems.push(format!(
                "line {}: criterion reported an error — `{line}` (a result line broken by it is invisible to the tracker)",
                index + 1
            ));
        }
        results.extend(parse_line(line));
    }
    if results.is_empty() {
        problems.push(
            "no `test <name> ... bench: <value> <unit> (+/- <range>)` line — github-action-benchmark would find no result"
                .to_owned(),
        );
    } else {
        for name in expected {
            if !results.iter().any(|result| result.name == *name) {
                problems.push(format!("no parseable result for `{name}`"));
            }
        }
    }
    if problems.is_empty() {
        Ok(results)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    // Tests assert on the parsed result directly; a panic is the failure.
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// The literal log of the failing nightly runs (issue #185).
    const BROKEN: &str = "test cold_start/open_and_first_page_50k ... Criterion.rs ERROR: error: Failed to access file \"/home/runner/work/curio-rss/curio-rss/target/criterion/cold_start/open_and_first_page_50k/base/sample.json\": No such file or directory (os error 2)\nbench:      439971 ns/iter (+/- 15703)\n\n";

    const CLEAN: &str =
        "test cold_start/open_and_first_page_50k ... bench:      836547 ns/iter (+/- 12949)\n\n";

    #[test]
    fn the_issue_185_log_is_rejected_with_its_cause() {
        let problems = check(BROKEN, &["cold_start/open_and_first_page_50k"]).unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(problems[0].starts_with("line 1: criterion reported an error"));
        assert!(problems[0].contains("base/sample.json"));
        assert!(problems[1].contains("would find no result"));
    }

    #[test]
    fn a_clean_bencher_line_parses_like_the_tracker() {
        let results = check(CLEAN, &["cold_start/open_and_first_page_50k"]).unwrap();
        assert_eq!(
            results,
            vec![BenchResult {
                name: "cold_start/open_and_first_page_50k".to_owned(),
                value: 836_547.0,
                unit: "ns/iter".to_owned(),
                range: "12949".to_owned(),
            }]
        );
    }

    #[test]
    fn commas_crlf_and_libtest_spacing_parse() {
        let results = check(
            "running 1 test\r\ntest bench_fib_20 ... bench:      37,174 ns/iter (+/- 7,527)\r\n",
            &[],
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "bench_fib_20");
        assert!((results[0].value - 37_174.0).abs() < f64::EPSILON);
        assert_eq!(results[0].range, "7527");
    }

    #[test]
    fn an_error_line_fails_even_beside_a_parseable_result() {
        let log = format!("Criterion.rs ERROR: error: something else\n{CLEAN}");
        let problems = check(&log, &[]).unwrap_err();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("something else"));
    }

    #[test]
    fn a_missing_expected_benchmark_is_named() {
        let problems = check(CLEAN, &["cold_start/other"]).unwrap_err();
        assert_eq!(
            problems,
            vec!["no parseable result for `cold_start/other`".to_owned()]
        );
    }

    #[test]
    fn empty_or_criterion_format_output_finds_nothing() {
        assert!(check("", &[]).is_err());
        // criterion's default (non-bencher) report is not what `tool: cargo`
        // parses.
        assert!(
            check(
                "cold_start/open_and_first_page_50k\n                        time:   [430.1 µs 439.9 µs 451.2 µs]\n",
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn lines_off_the_tracker_shape_are_skipped() {
        for line in [
            "test x ... ok",
            "test ... bench: 1 ns/iter (+/- 1)",
            "test x... bench: 1 ns/iter (+/- 1)",
            "test x ...bench: 1 ns/iter (+/- 1)",
            "test x ... bench:1 ns/iter (+/- 1)",
            "test x ... bench: 1x ns/iter (+/- 1)",
            "test x ... bench: 1 ns (+/- 1)",
            "test x ... bench: 1 ns/ (+/- 1)",
            "test x ... bench: 1 ns/iter (+/- 1",
            "test x ... bench: 1 ns/iter (+/- a)",
            "test x ... bench: 1 ns/iter (+/- 1) trailing",
            " test x ... bench: 1 ns/iter (+/- 1)",
        ] {
            assert_eq!(parse_line(line), None, "{line}");
        }
        // JavaScript's `\w` is ASCII-only, so the tracker skips `µs/iter`.
        assert_eq!(parse_line("test x ... bench: 1 µs/iter (+/- 1)"), None);
        assert!(parse_line("test x ... bench: 1.5 ms/iter (+/- 0.1)").is_some());
    }

    #[test]
    fn a_name_with_dots_runs_to_the_last_ellipsis_like_the_greedy_regex() {
        let result =
            parse_line("test group/wait ... then/load ... bench: 7 ns/iter (+/- 1)").unwrap();
        assert_eq!(result.name, "group/wait ... then/load");
        let result = parse_line("test a...b ... bench: 7 ns/iter (+/- 1)").unwrap();
        assert_eq!(result.name, "a...b");
    }
}
