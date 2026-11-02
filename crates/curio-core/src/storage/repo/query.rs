//! The search syntax's compiler: user input into an escaped FTS5
//! expression (the SQL around it is compiled in `listing`).

/// One searchable unit of a parsed query: a bare word or a quoted phrase.
#[derive(Debug, PartialEq, Eq)]
struct Atom {
    /// The literal text (quotes stripped; escaped only when rendered).
    text: String,
    /// Matched as a prefix (`"text"*`).
    prefix: bool,
    /// A `-` exclusion.
    negated: bool,
    /// A phrase whose closing `"` was typed — it is finished, so it never
    /// gets the implicit last-word prefix (an unterminated phrase is still
    /// being typed and does).
    closed: bool,
}

impl Atom {
    /// The atom as one FTS5 string (embedded quotes doubled) — always a
    /// valid FTS5 phrase, whatever the text holds.
    fn render(&self) -> String {
        let star = if self.prefix { "*" } else { "" };
        format!("\"{}\"{star}", self.text.replace('"', "\"\""))
    }
}

/// A lexed query item.
#[derive(Debug, PartialEq, Eq)]
enum Item {
    Atom(Atom),
    /// The upper-case `OR` operator.
    Or,
}

/// The implicit prefix on the last word needs this many characters: a
/// one-letter prefix expands to a large share of the vocabulary for
/// almost no narrowing.
const IMPLICIT_PREFIX_MIN_CHARS: usize = 2;

/// The length, in characters, of the token a trailing `*` on `text` would
/// widen: its last run of letters and digits. FTS5's tokenizer splits on
/// everything else, so the prefix lands on that token alone — `c++` and
/// `C#` index as the one-letter token `c`, `write ahe` ends in `ahe`.
fn last_token_chars(text: &str) -> usize {
    text.chars()
        .rev()
        .skip_while(|c| !c.is_alphanumeric())
        .take_while(|c| c.is_alphanumeric())
        .count()
}

/// Splits user input into atoms and `OR`s (see [`super::SearchArticles`]
/// for the syntax). Atoms without a single letter or digit are dropped —
/// they hold no token the index could match, and an empty phrase would
/// empty an AND of everything else.
fn lex_query(input: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut chars = input.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else {
            break;
        };
        let negated = first == '-';
        if negated {
            chars.next();
        }
        let quoted = chars.next_if_eq(&'"').is_some();
        let mut text = String::new();
        let mut prefix = false;
        let mut closed = false;
        if quoted {
            for c in chars.by_ref() {
                if c == '"' {
                    closed = true;
                    break;
                }
                text.push(c);
            }
            while chars.next_if_eq(&'*').is_some() {
                prefix = true;
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                text.push(c);
            }
            let body = text.trim_end_matches('*').len();
            prefix = body < text.len();
            text.truncate(body);
            if !negated && text == "OR" && !prefix {
                items.push(Item::Or);
                continue;
            }
        }
        if text.chars().any(char::is_alphanumeric) {
            items.push(Item::Atom(Atom {
                text,
                prefix,
                negated,
                closed,
            }));
        }
    }
    // Search-as-you-type: the word still being typed — a bare word, or the
    // last word of an unterminated phrase — matches as a prefix, unless the
    // token it widens is too short to narrow anything.
    if let Some(Item::Atom(last)) = items.last_mut()
        && !last.negated
        && !last.closed
        && last_token_chars(&last.text) >= IMPLICIT_PREFIX_MIN_CHARS
    {
        last.prefix = true;
    }
    items
}

/// Compiles user input into a safe FTS5 query (syntax on
/// [`super::SearchArticles`]): positive atoms become an AND of OR-groups,
/// exclusions one `NOT (… OR …)` — a single flat OR, so any number of
/// exclusions stays inside FTS5's expression-depth limit. Every atom is a
/// quoted FTS5 string, so input text never acts as FTS5 syntax. `None`
/// when nothing positive is left to match.
pub(super) fn fts_match_expr(input: &str) -> Option<String> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    let mut or_pending = false;
    for item in lex_query(input) {
        match item {
            Item::Or => or_pending = !groups.is_empty(),
            Item::Atom(atom) if atom.negated => {
                excluded.push(atom.render());
                or_pending = false;
            }
            Item::Atom(atom) => {
                match groups.last_mut() {
                    Some(group) if or_pending => group.push(atom.render()),
                    _ => groups.push(vec![atom.render()]),
                }
                or_pending = false;
            }
        }
    }
    if groups.is_empty() {
        return None;
    }
    let positive = groups
        .into_iter()
        .map(|group| {
            if group.len() == 1 {
                group.join("")
            } else {
                format!("({})", group.join(" OR "))
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    if excluded.is_empty() {
        Some(positive)
    } else {
        Some(format!("({positive}) NOT ({})", excluded.join(" OR ")))
    }
}

#[cfg(test)]
mod tests {
    // Test code: a panicking unwrap is the failure report.
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn fts_match_expr_quotes_every_token_and_prefixes_the_last() {
        assert_eq!(
            fts_match_expr("rust sqlite"),
            Some("\"rust\" AND \"sqlite\"*".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_keeps_quoted_phrases_whole() {
        assert_eq!(
            fts_match_expr("say \"hello world\""),
            Some("\"say\" AND \"hello world\"".to_owned()),
            "a phrase is one atom and never gets the implicit prefix"
        );
        assert_eq!(
            fts_match_expr("\"write ahe"),
            Some("\"write ahe\"*".to_owned()),
            "an unterminated quote runs to the end, its last word still being typed"
        );
        assert_eq!(
            fts_match_expr("\"write a"),
            Some("\"write a\"".to_owned()),
            "a one-letter last word in an open phrase is not widened"
        );
        assert_eq!(
            fts_match_expr("\"hello wor\"*"),
            Some("\"hello wor\"*".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_escapes_embedded_quotes() {
        assert_eq!(
            fts_match_expr("it\"s ok"),
            Some("\"it\"\"s\" AND \"ok\"*".to_owned()),
            "a quote inside a bare word is doubled, not a phrase start"
        );
    }

    #[test]
    fn fts_match_expr_explicit_and_implicit_prefixes() {
        assert_eq!(
            fts_match_expr("sql* tutorial"),
            Some("\"sql\"* AND \"tutorial\"*".to_owned())
        );
        assert_eq!(
            fts_match_expr("rust a"),
            Some("\"rust\" AND \"a\"".to_owned()),
            "a one-letter last word is not widened to a prefix"
        );
        assert_eq!(fts_match_expr("a*"), Some("\"a\"*".to_owned()));
        for (input, expr) in [
            ("c++", "\"c++\""),
            ("C#", "\"C#\""),
            ("a.", "\"a.\""),
            ("learn c++", "\"learn\" AND \"c++\""),
        ] {
            assert_eq!(
                fts_match_expr(input),
                Some(expr.to_owned()),
                "{input}: the tokenizer indexes a one-letter token, so no implicit prefix"
            );
        }
        assert_eq!(
            fts_match_expr("node.js"),
            Some("\"node.js\"*".to_owned()),
            "the prefix widens the last token, `js`"
        );
    }

    #[test]
    fn last_token_chars_counts_the_last_letter_or_digit_run() {
        assert_eq!(last_token_chars("rust"), 4);
        assert_eq!(last_token_chars("c++"), 1);
        assert_eq!(last_token_chars("C#"), 1);
        assert_eq!(last_token_chars("write ahe"), 3);
        assert_eq!(last_token_chars("v2.0"), 1);
        assert_eq!(last_token_chars("日本語"), 3);
        assert_eq!(last_token_chars("--"), 0);
    }

    #[test]
    fn fts_match_expr_compiles_exclusions_into_one_flat_not() {
        assert_eq!(
            fts_match_expr("rust -async -\"tokio runtime\""),
            Some("(\"rust\") NOT (\"async\" OR \"tokio runtime\")".to_owned()),
            "an exclusion is never prefixed, nor is a word before it"
        );
        assert_eq!(
            fts_match_expr("-only -exclusions"),
            None,
            "nothing positive to match"
        );
    }

    #[test]
    fn fts_match_expr_groups_or_tighter_than_and() {
        assert_eq!(
            fts_match_expr("rust OR go tutorial"),
            Some("(\"rust\" OR \"go\") AND \"tutorial\"*".to_owned())
        );
        assert_eq!(
            fts_match_expr("OR rust OR"),
            Some("\"rust\"".to_owned()),
            "a dangling OR is dropped (and the last item was the OR)"
        );
        assert_eq!(
            fts_match_expr("rust or go"),
            Some("\"rust\" AND \"or\" AND \"go\"*".to_owned()),
            "lower-case or is a plain word"
        );
        assert_eq!(
            fts_match_expr("\"OR\""),
            Some("\"OR\"".to_owned()),
            "a quoted OR is the word"
        );
    }

    #[test]
    fn fts_match_expr_drops_atoms_without_a_letter_or_digit() {
        assert_eq!(
            fts_match_expr("rust - * \"\" ( )"),
            Some("\"rust\"*".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_rejects_empty_input() {
        assert_eq!(fts_match_expr("   "), None);
        assert_eq!(fts_match_expr(""), None);
        assert_eq!(fts_match_expr("- * -- OR"), None);
    }

    /// Property suite for the FTS query builder (issues #133, #138): no
    /// user input compiles to a query FTS5 rejects, FTS5 keywords in the
    /// input are matched literally, and every plain word finds the row
    /// holding it.
    mod fts_match_expr_props {
        #![allow(clippy::unwrap_used)]

        use proptest::prelude::*;

        use super::super::{Item, fts_match_expr, lex_query};

        /// An FTS5 table shaped like `articles_fts` (same columns, default
        /// tokenizer), holding `rows`.
        fn index(rows: &[&str]) -> rusqlite::Connection {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(title, content_text);")
                .unwrap();
            for row in rows {
                conn.execute("INSERT INTO t(title, content_text) VALUES ('', ?1)", [row])
                    .unwrap();
            }
            conn
        }

        fn hits(conn: &rusqlite::Connection, expr: &str) -> rusqlite::Result<i64> {
            conn.query_row("SELECT count(*) FROM t WHERE t MATCH ?1", [expr], |row| {
                row.get(0)
            })
        }

        /// FTS5 query-language fragments a raw `MATCH` would interpret.
        const OPERATORS: &[&str] = &[
            "AND",
            "OR",
            "NOT",
            "NEAR",
            "NEAR(",
            "*",
            "^",
            "-",
            "+",
            ":",
            "(",
            ")",
            "\"",
            "\"\"",
            "title:",
            "{title content_text}:",
            "a*",
            "NEAR(a b, 2)",
            "'",
            ";",
            "--",
        ];

        proptest! {
            #[test]
            fn every_compiled_query_is_valid_fts5(input in "\\PC{0,120}") {
                let conn = index(&["rust sqlite query", "日本語 テキスト", "say \"hi\""]);
                match fts_match_expr(&input) {
                    // `None` only when nothing positive survived lexing.
                    None => prop_assert!(
                        !lex_query(&input)
                            .iter()
                            .any(|item| matches!(item, Item::Atom(atom) if !atom.negated)),
                        "{input:?}"
                    ),
                    Some(expr) => {
                        let result = hits(&conn, &expr);
                        prop_assert!(result.is_ok(), "{input:?} → {expr:?}: {result:?}");
                    }
                }
            }

            #[test]
            fn operator_syntax_never_breaks_the_query(
                words in prop::collection::vec(prop::sample::select(OPERATORS), 1..6),
            ) {
                // Raw, most of these are FTS5 syntax errors (`NEAR(`, a
                // lone `"`, `title:` with no term) or operators. Compiled,
                // a fragment is a quoted atom, Curio's own `OR`/`-`/`*`, or
                // dropped (no letter or digit): always a valid query.
                let conn = index(&["a apple and or not near", "title content"]);
                if let Some(expr) = fts_match_expr(&words.join(" ")) {
                    let result = hits(&conn, &expr);
                    prop_assert!(result.is_ok(), "{words:?} → {expr:?}: {result:?}");
                }
            }

            #[test]
            fn fts5_keywords_are_matched_literally(
                words in prop::collection::vec(
                    prop::sample::select(&["AND", "NOT", "NEAR", "title:", "a"][..]),
                    1..6,
                ),
            ) {
                // Every word holds a letter, so each survives as a quoted
                // atom: the compiled query is a pure AND of literals, never
                // an FTS5 operator, and matches the row holding them all.
                let conn = index(&["a and not near title", "unrelated filler"]);
                let expr = fts_match_expr(&words.join(" ")).unwrap();
                prop_assert!(!expr.contains(" OR ") && !expr.contains(" NOT "), "{expr}");
                prop_assert_eq!(hits(&conn, &expr).unwrap(), 1);
            }

            #[test]
            fn any_number_of_terms_stays_within_fts5_limits(
                count in 1_usize..400,
                negated in any::<bool>(),
                ored in any::<bool>(),
            ) {
                // FTS5 caps expression depth at 256: exclusions compile to
                // one flat `NOT (… OR …)`, never a chain of NOTs.
                let conn = index(&["w0 body"]);
                let words: Vec<String> = (0..count)
                    .map(|i| if negated { format!("-x{i}") } else { format!("w{i}") })
                    .collect();
                let joiner = if ored && !negated { " OR " } else { " " };
                let input = format!("w0 {}", words.join(joiner));
                let expr = fts_match_expr(&input).unwrap();
                let result = hits(&conn, &expr);
                prop_assert!(result.is_ok(), "{count} terms: {result:?}");
            }

            #[test]
            fn every_plain_word_finds_its_row(
                words in prop::collection::vec("[a-z]{2,10}", 1..5),
            ) {
                let row = words.join(" ");
                // Digits only: no lower-case prefix can reach the filler.
                let conn = index(&[&row, "12345 67890"]);
                let expr = fts_match_expr(&format!("  {}\t", words.join("  "))).unwrap();
                prop_assert_eq!(expr.matches('"').count(), words.len() * 2);
                prop_assert_eq!(hits(&conn, &expr).unwrap(), 1);
            }
        }
    }
}
