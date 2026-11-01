//! [`Node`] tree → YAML text: libyaml's block emitter, ported for the
//! subset frontmatter uses (no tags, no anchors, no flow collections
//! beyond `[]`/`{}`, unbounded line width), with its column bookkeeping
//! intact so indentation and line breaks land on the same bytes.

use super::ser::{Node, Style};

/// libyaml's `best_indent`.
const INDENT: usize = 2;

/// libyaml's limit on an implicit (`key: value`) key, in bytes.
const MAX_SIMPLE_KEY_LEN: usize = 128;

pub(super) fn emit(root: &Node) -> String {
    let mut emitter = Emitter {
        out: String::with_capacity(512),
        column: 0,
        indent: None,
        indents: Vec::new(),
        whitespace: true,
        indention: true,
    };
    emitter.node(root, Context::default());
    // Document end: close the last line.
    emitter.write_indent();
    emitter.out
}

/// Where a node sits — libyaml's `*_context` flags.
#[derive(Debug, Clone, Copy, Default)]
struct Context {
    mapping: bool,
    simple_key: bool,
}

/// The final scalar style, after the emitter's checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chosen {
    Plain,
    SingleQuoted,
    DoubleQuoted,
    Literal,
}

/// What a scalar's content permits (`yaml_emitter_analyze_scalar`).
#[allow(clippy::struct_excessive_bools)] // libyaml's `scalar_data` flags, one to one
#[derive(Debug, Clone, Copy)]
struct Analysis {
    multiline: bool,
    block_plain_allowed: bool,
    single_quoted_allowed: bool,
    block_allowed: bool,
}

/// libyaml's `IS_PRINTABLE` (with unicode output on).
fn is_printable(c: char) -> bool {
    matches!(c,
        '\n' | '\u{20}'..='\u{7E}' | '\u{A0}'..='\u{D7FF}' | '\u{10000}'..='\u{10FFFF}')
        || (('\u{E000}'..='\u{FFFD}').contains(&c) && c != '\u{FEFF}')
}

/// libyaml's `IS_BREAK`.
fn is_break(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// libyaml's `IS_BLANKZ`; `None` is the end of the string (its NUL).
fn is_blankz(c: Option<char>) -> bool {
    c.is_none_or(|c| matches!(c, ' ' | '\t' | '\0') || is_break(c))
}

fn analyze(value: &str) -> Analysis {
    let chars: Vec<char> = value.chars().collect();
    if chars.is_empty() {
        return Analysis {
            multiline: false,
            block_plain_allowed: true,
            single_quoted_allowed: true,
            block_allowed: false,
        };
    }
    let mut block_indicators = value.starts_with("---") || value.starts_with("...");
    let (mut line_breaks, mut special_characters) = (false, false);
    let (mut leading_space, mut leading_break) = (false, false);
    let (mut trailing_space, mut trailing_break) = (false, false);
    let (mut break_space, mut space_break) = (false, false);
    let (mut previous_space, mut previous_break) = (false, false);
    let last = chars.len() - 1;
    for (i, &c) in chars.iter().enumerate() {
        let preceded_by_whitespace = i == 0 || is_blankz(Some(chars[i - 1]));
        let followed_by_whitespace = is_blankz(chars.get(i + 1).copied());
        if i == 0 {
            if "#,[]{}&*!|>'\"%@`".contains(c) || (c == '-' && followed_by_whitespace) {
                block_indicators = true;
            }
            if matches!(c, '?' | ':') && followed_by_whitespace {
                block_indicators = true;
            }
        } else if (c == ':' && followed_by_whitespace) || (c == '#' && preceded_by_whitespace) {
            block_indicators = true;
        }
        if !is_printable(c) {
            special_characters = true;
        }
        if is_break(c) {
            line_breaks = true;
        }
        if c == ' ' {
            leading_space |= i == 0;
            trailing_space |= i == last;
            break_space |= previous_break;
            (previous_space, previous_break) = (true, false);
        } else if is_break(c) {
            leading_break |= i == 0;
            trailing_break |= i == last;
            space_break |= previous_space;
            (previous_space, previous_break) = (false, true);
        } else {
            (previous_space, previous_break) = (false, false);
        }
    }
    let mut analysis = Analysis {
        multiline: line_breaks,
        block_plain_allowed: true,
        single_quoted_allowed: true,
        block_allowed: true,
    };
    if leading_space || leading_break || trailing_space || trailing_break {
        analysis.block_plain_allowed = false;
    }
    if trailing_space {
        analysis.block_allowed = false;
    }
    if break_space {
        analysis.block_plain_allowed = false;
        analysis.single_quoted_allowed = false;
    }
    if space_break || special_characters {
        analysis.block_plain_allowed = false;
        analysis.single_quoted_allowed = false;
        analysis.block_allowed = false;
    }
    if line_breaks || block_indicators {
        analysis.block_plain_allowed = false;
    }
    analysis
}

/// `yaml_emitter_select_scalar_style` for block context.
fn select_style(value: &str, requested: Style, analysis: Analysis, simple_key: bool) -> Chosen {
    let mut style = match requested {
        Style::Any | Style::Plain => Chosen::Plain,
        Style::SingleQuoted => Chosen::SingleQuoted,
        Style::Literal => Chosen::Literal,
    };
    if simple_key && analysis.multiline {
        style = Chosen::DoubleQuoted;
    }
    if style == Chosen::Plain && (!analysis.block_plain_allowed || (value.is_empty() && simple_key))
    {
        style = Chosen::SingleQuoted;
    }
    if style == Chosen::SingleQuoted && !analysis.single_quoted_allowed {
        style = Chosen::DoubleQuoted;
    }
    if style == Chosen::Literal && (!analysis.block_allowed || simple_key) {
        style = Chosen::DoubleQuoted;
    }
    style
}

/// The emitter state libyaml threads through every write.
#[derive(Debug)]
struct Emitter {
    out: String,
    /// Characters written on the current line.
    column: usize,
    /// The current indentation; `None` before the root collection.
    indent: Option<usize>,
    indents: Vec<Option<usize>>,
    /// The last character written was whitespace.
    whitespace: bool,
    /// Only indentation has been written on the current line.
    indention: bool,
}

impl Emitter {
    fn put(&mut self, c: char) {
        self.out.push(c);
        self.column += 1;
    }

    fn put_break(&mut self) {
        self.out.push('\n');
        self.column = 0;
    }

    /// libyaml's `WRITE_BREAK`: `\n` as the line break, any other break
    /// character copied as-is (it still ends the line).
    fn write_break(&mut self, c: char) {
        if c == '\n' {
            self.put_break();
        } else {
            self.out.push(c);
            self.column = 0;
        }
    }

    fn increase_indent(&mut self, flow: bool, indentless: bool) {
        self.indents.push(self.indent);
        self.indent = Some(match self.indent {
            None if flow => INDENT,
            None => 0,
            Some(indent) if indentless => indent,
            Some(indent) => indent + INDENT,
        });
    }

    fn pop_indent(&mut self) {
        self.indent = self.indents.pop().flatten();
    }

    fn write_indent(&mut self) {
        let indent = self.indent.unwrap_or(0);
        if !self.indention || self.column > indent || (self.column == indent && !self.whitespace) {
            self.put_break();
        }
        while self.column < indent {
            self.put(' ');
        }
        self.whitespace = true;
        self.indention = true;
    }

    fn write_indicator(
        &mut self,
        indicator: &str,
        need_whitespace: bool,
        is_whitespace: bool,
        is_indention: bool,
    ) {
        if need_whitespace && !self.whitespace {
            self.put(' ');
        }
        for c in indicator.chars() {
            self.put(c);
        }
        self.whitespace = is_whitespace;
        self.indention = self.indention && is_indention;
    }

    fn node(&mut self, node: &Node, context: Context) {
        match node {
            Node::Scalar(value, style) => self.scalar(value, *style, context),
            Node::Seq(items) if items.is_empty() => self.empty_collection("[", "]"),
            Node::Map(entries) if entries.is_empty() => self.empty_collection("{", "}"),
            Node::Seq(items) => self.block_sequence(items, context),
            Node::Map(entries) => self.block_mapping(entries),
        }
    }

    fn empty_collection(&mut self, open: &str, close: &str) {
        self.write_indicator(open, true, true, false);
        self.write_indicator(close, false, false, false);
    }

    fn block_sequence(&mut self, items: &[Node], context: Context) {
        // Directly under a mapping key (`key:` then newline), items sit
        // flush with the key — libyaml's "indentless" sequence.
        self.increase_indent(false, context.mapping && !self.indention);
        for item in items {
            self.write_indent();
            self.write_indicator("-", true, false, true);
            self.node(item, Context::default());
        }
        self.pop_indent();
    }

    fn block_mapping(&mut self, entries: &[(Node, Node)]) {
        self.increase_indent(false, false);
        let value_context = Context {
            mapping: true,
            simple_key: false,
        };
        for (key, value) in entries {
            self.write_indent();
            if is_simple_key(key) {
                self.node(
                    key,
                    Context {
                        mapping: true,
                        simple_key: true,
                    },
                );
                self.write_indicator(":", false, false, false);
            } else {
                self.write_indicator("?", true, false, true);
                self.node(key, value_context);
                self.write_indent();
                self.write_indicator(":", true, false, true);
            }
            self.node(value, value_context);
        }
        self.pop_indent();
    }

    fn scalar(&mut self, value: &str, requested: Style, context: Context) {
        let style = select_style(value, requested, analyze(value), context.simple_key);
        self.increase_indent(true, false);
        match style {
            Chosen::Plain => self.plain(value),
            Chosen::SingleQuoted => self.single_quoted(value),
            Chosen::DoubleQuoted => self.double_quoted(value),
            Chosen::Literal => self.literal(value),
        }
        self.pop_indent();
    }

    fn plain(&mut self, value: &str) {
        if !self.whitespace && !value.is_empty() {
            self.put(' ');
        }
        // Plain style is never chosen for content with line breaks, and
        // the unbounded width never folds at a space.
        for c in value.chars() {
            self.put(c);
        }
        self.whitespace = false;
        self.indention = false;
    }

    fn single_quoted(&mut self, value: &str) {
        self.write_indicator("'", true, false, false);
        let mut breaks = false;
        for c in value.chars() {
            if c == ' ' {
                self.put(c);
            } else if is_break(c) {
                if !breaks && c == '\n' {
                    self.put_break();
                }
                self.write_break(c);
                self.indention = true;
                breaks = true;
            } else {
                if breaks {
                    self.write_indent();
                }
                if c == '\'' {
                    self.put('\'');
                }
                self.put(c);
                self.indention = false;
                breaks = false;
            }
        }
        if breaks {
            self.write_indent();
        }
        self.write_indicator("'", false, false, false);
        self.whitespace = false;
        self.indention = false;
    }

    fn double_quoted(&mut self, value: &str) {
        self.write_indicator("\"", true, false, false);
        for c in value.chars() {
            if !is_printable(c) || c == '\u{FEFF}' || is_break(c) || c == '"' || c == '\\' {
                self.escape(c);
            } else {
                self.put(c);
            }
        }
        self.write_indicator("\"", false, false, false);
        self.whitespace = false;
        self.indention = false;
    }

    fn escape(&mut self, c: char) {
        self.put('\\');
        let short = match c {
            '\0' => Some('0'),
            '\u{07}' => Some('a'),
            '\u{08}' => Some('b'),
            '\t' => Some('t'),
            '\n' => Some('n'),
            '\u{0B}' => Some('v'),
            '\u{0C}' => Some('f'),
            '\r' => Some('r'),
            '\u{1B}' => Some('e'),
            '"' => Some('"'),
            '\\' => Some('\\'),
            '\u{85}' => Some('N'),
            '\u{A0}' => Some('_'),
            '\u{2028}' => Some('L'),
            '\u{2029}' => Some('P'),
            _ => None,
        };
        if let Some(short) = short {
            self.put(short);
            return;
        }
        let code = u32::from(c);
        let (prefix, width) = match code {
            0..=0xFF => ('x', 2),
            0x100..=0xFFFF => ('u', 4),
            _ => ('U', 8),
        };
        self.put(prefix);
        for c in format!("{code:0width$X}").chars() {
            self.put(c);
        }
    }

    fn literal(&mut self, value: &str) {
        self.write_indicator("|", true, false, false);
        self.block_scalar_hints(value);
        self.put_break();
        self.indention = true;
        self.whitespace = true;
        let mut breaks = true;
        for c in value.chars() {
            if is_break(c) {
                self.write_break(c);
                self.indention = true;
                breaks = true;
            } else {
                if breaks {
                    self.write_indent();
                }
                self.put(c);
                self.indention = false;
                breaks = false;
            }
        }
    }

    /// The indentation indicator (content starting with a space or break)
    /// and the chomping indicator (`-` strip, none clip, `+` keep).
    fn block_scalar_hints(&mut self, value: &str) {
        let mut chars = value.chars();
        if chars.next().is_some_and(|c| c == ' ' || is_break(c)) {
            self.write_indicator(&INDENT.to_string(), false, false, false);
        }
        let mut reversed = value.chars().rev();
        let chomp = match (reversed.next(), reversed.next()) {
            (None, _) => Some("-"),
            (Some(last), _) if !is_break(last) => Some("-"),
            (Some(_), None) => Some("+"),
            (Some(_), Some(before)) if is_break(before) => Some("+"),
            (Some(_), Some(_)) => None,
        };
        if let Some(chomp) = chomp {
            self.write_indicator(chomp, false, false, false);
        }
    }
}

/// `yaml_emitter_check_simple_key`: a short single-line scalar or an
/// empty collection.
fn is_simple_key(key: &Node) -> bool {
    match key {
        Node::Scalar(value, _) => !analyze(value).multiline && value.len() <= MAX_SIMPLE_KEY_LEN,
        Node::Seq(items) => items.is_empty(),
        Node::Map(entries) => entries.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str, style: Style) -> Node {
        Node::Scalar(value.to_owned(), style)
    }

    #[test]
    fn root_scalars_and_sequences_follow_libyaml() {
        assert_eq!(emit(&s("x", Style::Any)), "x\n");
        assert_eq!(emit(&Node::Seq(vec![])), "[]\n");
        assert_eq!(
            emit(&Node::Seq(vec![
                s("a", Style::Any),
                Node::Seq(vec![s("b", Style::Any)])
            ])),
            "- a\n- - b\n"
        );
    }

    #[test]
    fn collection_keys_use_the_explicit_form_unless_empty() {
        let map = Node::Map(vec![
            (Node::Seq(vec![]), s("empty", Style::Any)),
            (Node::Seq(vec![s("k", Style::Any)]), s("v", Style::Any)),
            (
                Node::Map(vec![(s("a", Style::Any), s("b", Style::Any))]),
                s("w", Style::Any),
            ),
        ]);
        assert_eq!(emit(&map), "[]: empty\n? - k\n: v\n? a: b\n: w\n");
    }

    #[test]
    fn printable_breaks_inside_quotes_are_copied_and_re_indented() {
        // U+2028 is printable but a line break to libyaml: no plain style,
        // single quotes keep it raw and re-indent what follows.
        let map = Node::Map(vec![(s("k", Style::Any), s("a\u{2028}b", Style::Any))]);
        assert_eq!(emit(&map), "k: 'a\u{2028}  b'\n");
        let trailing = Node::Map(vec![(s("k", Style::Any), s("a\u{2029}", Style::Any))]);
        assert_eq!(emit(&trailing), "k: 'a\u{2029}  '\n");
        // A space next to the break rules single quotes out.
        let spaced = Node::Map(vec![(s("k", Style::Any), s("a \u{2029}", Style::Any))]);
        assert_eq!(emit(&spaced), "k: \"a \\P\"\n");
        // A key with a line break is never an implicit key.
        let key = Node::Map(vec![(s("a\u{2028}b", Style::Any), s("v", Style::Any))]);
        assert_eq!(emit(&key), "? 'a\u{2028}  b'\n: v\n");
    }

    #[test]
    fn escapes_cover_every_width() {
        let map = Node::Map(vec![(
            s("k", Style::Any),
            s("\u{1}\u{FFFE}\u{FEFF}\u{A0}\t\u{10FFFE}\u{85}", Style::Any),
        )]);
        // U+A0 is printable: written raw, never as `\_`.
        assert_eq!(
            emit(&map),
            "k: \"\\x01\\uFFFE\\uFEFF\u{A0}\\t\u{10FFFE}\\N\"\n"
        );
    }
}
