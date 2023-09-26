use crate::lex::lex;
use crate::SyntaxKind;
use crate::SyntaxKind::*;
use chrono::{DateTime, FixedOffset};
use debversion::Version;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Urgency {
    Low,
    Medium,
    High,
    Emergency,
    Critical,
}

impl ToString for Urgency {
    fn to_string(&self) -> String {
        match self {
            Urgency::Low => "low".to_string(),
            Urgency::Medium => "medium".to_string(),
            Urgency::High => "high".to_string(),
            Urgency::Emergency => "emergency".to_string(),
            Urgency::Critical => "critical".to_string(),
        }
    }
}

impl FromStr for Urgency {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Urgency::Low),
            "medium" => Ok(Urgency::Medium),
            "high" => Ok(Urgency::High),
            "emergency" => Ok(Urgency::Emergency),
            "critical" => Ok(Urgency::Critical),
            _ => Err(ParseError(vec![format!("invalid urgency: {}", s)])),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParseError(Vec<String>);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for err in &self.0 {
            writeln!(f, "{}", err)?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

/// Second, implementing the `Language` trait teaches rowan to convert between
/// these two SyntaxKind types, allowing for a nicer SyntaxNode API where
/// "kinds" are values from our `enum SyntaxKind`, instead of plain u16 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Lang {}
impl rowan::Language for Lang {
    type Kind = SyntaxKind;
    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }
    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

/// GreenNode is an immutable tree, which is cheap to change,
/// but doesn't contain offsets and parent pointers.
use rowan::GreenNode;

/// You can construct GreenNodes by hand, but a builder
/// is helpful for top-down parsers: it maintains a stack
/// of currently in-progress nodes
use rowan::GreenNodeBuilder;

/// The parse results are stored as a "green tree".
/// We'll discuss working with the results later
#[derive(Debug)]
struct Parse {
    green_node: GreenNode,
    #[allow(unused)]
    errors: Vec<String>,
}

fn parse(text: &str) -> Parse {
    struct Parser {
        /// input tokens, including whitespace,
        /// in *reverse* order.
        tokens: Vec<(SyntaxKind, String)>,
        /// the in-progress tree.
        builder: GreenNodeBuilder<'static>,
        /// the list of syntax errors we've accumulated
        /// so far.
        errors: Vec<String>,
    }

    impl Parser {
        fn error(&mut self, msg: String) {
            self.builder.start_node(ERROR.into());
            if self.current().is_some() {
                self.bump();
            }
            self.errors.push(msg);
            self.builder.finish_node();
        }

        fn parse_entry_header(&mut self) {
            self.builder.start_node(ENTRY_HEADER.into());
            self.expect(IDENTIFIER);

            self.skip_ws();

            self.expect(VERSION);

            self.builder.start_node(DISTRIBUTIONS.into());
            loop {
                self.skip_ws();

                match self.current() {
                    Some(IDENTIFIER) => self.bump(),
                    Some(SEMICOLON) => {
                        break;
                    }
                    _ => {
                        self.error("expected distribution or semicolon".to_string());
                        break;
                    }
                }
            }
            self.builder.finish_node();

            self.builder.start_node(METADATA.into());
            if self.current() == Some(SEMICOLON) {
                self.bump();
                loop {
                    self.skip_ws();

                    if self.current() == Some(NEWLINE) {
                        break;
                    }

                    self.builder.start_node(METADATA_ENTRY.into());
                    if self.current() == Some(IDENTIFIER) {
                        self.builder.start_node(METADATA_KEY.into());
                        self.bump();
                        self.builder.finish_node();
                    } else {
                        self.error("expected metadata key".to_string());
                        self.builder.finish_node();
                        break;
                    }

                    if self.current() == Some(EQUALS) {
                        self.bump();
                    } else {
                        self.error("expected equals".to_string());
                        self.builder.finish_node();
                        break;
                    }

                    if self.current() == Some(IDENTIFIER) {
                        self.builder.start_node(METADATA_VALUE.into());
                        self.bump();
                        self.builder.finish_node();
                    } else {
                        self.error("expected metadata value".to_string());
                        self.builder.finish_node();
                        break;
                    }
                    self.builder.finish_node();
                }
            } else if self.current() == Some(NEWLINE) {
            } else {
                self.error("expected semicolon or newline".to_string());
            }
            self.builder.finish_node();

            self.expect(NEWLINE);
            self.builder.finish_node();
        }

        fn parse_entry(&mut self) {
            self.builder.start_node(ENTRY.into());
            self.parse_entry_header();
            loop {
                match self
                    .tokens
                    .last()
                    .map(|(kind, token)| (kind, token.as_str()))
                {
                    None => {
                        self.error("unexpected end of file".to_string());
                        break;
                    }
                    // empty line
                    Some((NEWLINE, _)) => {
                        self.builder.start_node(EMPTY_LINE.into());
                        self.bump();
                        self.builder.finish_node();
                    }
                    // details
                    Some((INDENT, "  ")) => {
                        self.parse_entry_detail();
                    }
                    // footer
                    Some((INDENT, " -- ")) => {
                        self.parse_entry_footer();
                        break;
                    }
                    _ => break,
                }
            }

            self.builder.finish_node();
        }

        pub fn parse_entry_detail(&mut self) {
            self.builder.start_node(ENTRY_BODY.into());
            self.expect(INDENT);

            match self.current() {
                Some(DETAIL) => {
                    self.bump();
                }
                Some(NEWLINE) => {}
                _ => {
                    self.error("expected detail".to_string());
                }
            }

            self.expect(NEWLINE);
            self.builder.finish_node();
        }

        pub fn parse_entry_footer(&mut self) {
            self.builder.start_node(ENTRY_FOOTER.into());

            if self.current() != Some(INDENT) {
                self.error("expected indent".to_string());
            } else {
                let dashes = &self.tokens.last().unwrap().1;
                if dashes != " -- " {
                    self.error("expected --".to_string());
                } else {
                    self.bump();
                }
            }

            self.builder.start_node(MAINTAINER.into());
            while self.current() == Some(TEXT)
                || (self.current() == Some(WHITESPACE) && self.next() != Some(EMAIL))
            {
                self.bump();
            }
            self.builder.finish_node();

            self.expect(WHITESPACE);

            self.expect(EMAIL);

            self.expect(WHITESPACE);

            self.builder.start_node(TIMESTAMP.into());
            loop {
                if self.current() != Some(TEXT) && self.current() != Some(WHITESPACE) {
                    break;
                }
                self.bump();
            }
            self.builder.finish_node();

            self.expect(NEWLINE);
            self.builder.finish_node();
        }

        fn parse(mut self) -> Parse {
            self.builder.start_node(ROOT.into());
            loop {
                match self.current() {
                    None => break,
                    Some(NEWLINE) => {
                        self.builder.start_node(EMPTY_LINE.into());
                        self.bump();
                        self.builder.finish_node();
                    }
                    Some(COMMENT) => {
                        self.bump();
                    }
                    Some(IDENTIFIER) => {
                        self.parse_entry();
                    }
                    t => {
                        self.error(format!("unexpected token {:?}", t));
                        break;
                    }
                }
            }
            // Close the root node.
            self.builder.finish_node();

            // Turn the builder into a GreenNode
            Parse {
                green_node: self.builder.finish(),
                errors: self.errors,
            }
        }
        /// Advance one token, adding it to the current branch of the tree builder.
        fn bump(&mut self) {
            let (kind, text) = self.tokens.pop().unwrap();
            self.builder.token(kind.into(), text.as_str());
        }
        /// Peek at the first unprocessed token
        fn current(&self) -> Option<SyntaxKind> {
            self.tokens.last().map(|(kind, _)| *kind)
        }

        fn next(&self) -> Option<SyntaxKind> {
            self.tokens
                .get(self.tokens.len() - 2)
                .map(|(kind, _)| *kind)
        }

        fn expect(&mut self, expected: SyntaxKind) {
            if self.current() != Some(expected) {
                self.error(format!("expected {:?}, got {:?}", expected, self.current()));
            } else {
                self.bump();
            }
        }
        fn skip_ws(&mut self) {
            while self.current() == Some(WHITESPACE) {
                self.bump()
            }
        }
    }

    let mut tokens = lex(text);
    tokens.reverse();
    Parser {
        tokens,
        builder: GreenNodeBuilder::new(),
        errors: Vec::new(),
    }
    .parse()
}

/// To work with the parse results we need a view into the
/// green tree - the Syntax tree.
/// It is also immutable, like a GreenNode,
/// but it contains parent pointers, offsets, and
/// has identity semantics.

type SyntaxNode = rowan::SyntaxNode<Lang>;
#[allow(unused)]
type SyntaxToken = rowan::SyntaxToken<Lang>;
#[allow(unused)]
type SyntaxElement = rowan::NodeOrToken<SyntaxNode, SyntaxToken>;

impl Parse {
    fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green_node.clone())
    }

    fn root(&self) -> ChangeLog {
        ChangeLog::cast(self.syntax()).unwrap()
    }
}

macro_rules! ast_node {
    ($ast:ident, $kind:ident) => {
        #[derive(PartialEq, Eq, Hash)]
        #[repr(transparent)]
        pub struct $ast(SyntaxNode);
        impl $ast {
            #[allow(unused)]
            fn cast(node: SyntaxNode) -> Option<Self> {
                if node.kind() == $kind {
                    Some(Self(node))
                } else {
                    None
                }
            }
        }

        impl ToString for $ast {
            fn to_string(&self) -> String {
                self.0.text().to_string()
            }
        }
    };
}

ast_node!(ChangeLog, ROOT);
ast_node!(Entry, ENTRY);
ast_node!(EntryHeader, ENTRY_HEADER);
ast_node!(EntryBody, ENTRY_BODY);
ast_node!(EntryFooter, ENTRY_FOOTER);
ast_node!(Maintainer, MAINTAINER);
ast_node!(Timestamp, TIMESTAMP);
ast_node!(MetadataEntry, METADATA_ENTRY);
ast_node!(MetadataKey, METADATA_KEY);
ast_node!(MetadataValue, METADATA_VALUE);

impl MetadataEntry {
    pub fn key(&self) -> Option<String> {
        self.0
            .children()
            .find_map(MetadataKey::cast)
            .map(|k| k.to_string())
    }

    pub fn value(&self) -> Option<String> {
        self.0
            .children()
            .find_map(MetadataValue::cast)
            .map(|k| k.to_string())
    }
}

impl ChangeLog {
    pub fn new() -> ChangeLog {
        let mut builder = GreenNodeBuilder::new();

        builder.start_node(ROOT.into());
        builder.finish_node();
        ChangeLog(SyntaxNode::new_root(builder.finish()))
    }

    /// Returns an iterator over all entries in the watch file.
    pub fn entries(&self) -> impl Iterator<Item = Entry> + '_ {
        self.0.children().filter_map(Entry::cast)
    }

    pub fn read_path(path: impl AsRef<std::path::Path>) -> Result<ChangeLog, std::io::Error> {
        let mut file = std::fs::File::open(path)?;
        Self::read(&mut file)
    }

    pub fn read<R: std::io::Read>(mut r: R) -> Result<ChangeLog, std::io::Error> {
        let mut buf = String::new();
        r.read_to_string(&mut buf)?;
        Ok(buf.parse().unwrap())
    }
}

impl Default for ChangeLog {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for ChangeLog {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = parse(s);
        if parsed.errors.is_empty() {
            Ok(parsed.root())
        } else {
            Err(ParseError(parsed.errors))
        }
    }
}

impl EntryHeader {
    pub fn version(&self) -> Option<Version> {
        self.0.children_with_tokens().find_map(|it| {
            if let Some(token) = it.as_token() {
                if token.kind() == VERSION {
                    let text = token.text()[1..token.text().len() - 1].to_string();
                    return Some(text.parse().unwrap());
                }
            }
            None
        })
    }

    pub fn package(&self) -> Option<String> {
        self.0.children_with_tokens().find_map(|it| {
            if let Some(token) = it.as_token() {
                if token.kind() == IDENTIFIER {
                    return Some(token.text().to_string());
                }
            }
            None
        })
    }

    pub fn distributions(&self) -> Option<Vec<String>> {
        let node = self.0.children().find(|it| it.kind() == DISTRIBUTIONS);

        node.map(|node| {
            node.children_with_tokens()
                .filter_map(|it| {
                    if let Some(token) = it.as_token() {
                        if token.kind() == IDENTIFIER {
                            return Some(token.text().to_string());
                        }
                    }
                    None
                })
                .collect::<Vec<_>>()
        })
    }

    fn metadata_node(&self) -> impl Iterator<Item = MetadataEntry> + '_ {
        let node = self.0.children().find(|it| it.kind() == METADATA);

        node.into_iter().flat_map(|node| {
            node.children_with_tokens()
                .filter_map(|it| MetadataEntry::cast(it.into_node()?))
        })
    }

    pub fn metadata(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.metadata_node().filter_map(|entry| {
            if let (Some(key), Some(value)) = (entry.key(), entry.value()) {
                Some((key, value))
            } else {
                None
            }
        })
    }

    pub fn urgency(&self) -> Option<Urgency> {
        for (key, value) in self.metadata() {
            if key.as_str() == "urgency" {
                return Some(value.parse().unwrap());
            }
        }
        None
    }
}

impl EntryFooter {
    pub fn email(&self) -> Option<String> {
        self.0.children_with_tokens().find_map(|it| {
            if let Some(token) = it.as_token() {
                let text = token.text();
                if token.kind() == EMAIL {
                    return Some(text[1..text.len() - 1].to_string());
                }
            }
            None
        })
    }

    pub fn maintainer(&self) -> Option<String> {
        self.0
            .children()
            .find_map(Maintainer::cast)
            .map(|m| m.text())
    }

    pub fn timestamp(&self) -> Option<String> {
        self.0
            .children()
            .find_map(Timestamp::cast)
            .map(|m| m.text())
    }
}

impl EntryBody {
    fn text(&self) -> String {
        self.0
            .children_with_tokens()
            .filter_map(|it| {
                if let Some(token) = it.as_token() {
                    if token.kind() == DETAIL {
                        return Some(token.text().to_string());
                    }
                }
                None
            })
            .collect::<Vec<_>>()
            .concat()
    }
}

impl Timestamp {
    fn text(&self) -> String {
        self.0.text().to_string()
    }
}

impl Maintainer {
    fn text(&self) -> String {
        self.0.text().to_string()
    }
}

impl Entry {
    fn header(&self) -> Option<EntryHeader> {
        self.0.children().find_map(EntryHeader::cast)
    }

    fn footer(&self) -> Option<EntryFooter> {
        self.0.children().find_map(EntryFooter::cast)
    }

    pub fn package(&self) -> Option<String> {
        self.header().and_then(|h| h.package())
    }

    pub fn version(&self) -> Option<Version> {
        self.header().and_then(|h| h.version())
    }

    pub fn distributions(&self) -> Option<Vec<String>> {
        self.header().and_then(|h| h.distributions())
    }

    pub fn changes(&self) -> impl Iterator<Item = EntryBody> + '_ {
        self.0.children().filter_map(EntryBody::cast)
    }

    pub fn email(&self) -> Option<String> {
        self.footer().and_then(|f| f.email())
    }

    pub fn maintainer(&self) -> Option<String> {
        self.footer().and_then(|f| f.maintainer())
    }

    pub fn timestamp(&self) -> Option<String> {
        self.footer().and_then(|f| f.timestamp())
    }

    pub fn datetime(&self) -> Option<DateTime<FixedOffset>> {
        self.timestamp().and_then(|ts| parse_time_string(&ts).ok())
    }

    pub fn urgency(&self) -> Option<Urgency> {
        self.header().and_then(|h| h.urgency())
    }

    pub fn change_lines(&self) -> impl Iterator<Item = String> + '_ {
        // TODO: empty head and tail empty lines
        self.0.children().filter_map(|n| {
            if let Some(ref change) = EntryBody::cast(n.clone()) {
                Some(change.text())
            } else if n.kind() == EMPTY_LINE {
                Some("".to_string())
            } else {
                None
            }
        })
    }
}

const CHANGELOG_TIME_FORMAT: &str = "%a, %d %b %Y %H:%M:%S %z";

fn parse_time_string(time_str: &str) -> Result<DateTime<FixedOffset>, chrono::ParseError> {
    DateTime::parse_from_str(time_str, CHANGELOG_TIME_FORMAT)
}

#[test]
fn test_parse_simple() {
    const CHANGELOG: &str = r#"breezy (3.3.4-1) unstable; urgency=low

  * New upstream release.

 -- Jelmer Vernooĳ <jelmer@debian.org>  Mon, 04 Sep 2023 18:13:45 -0500

breezy (3.3.3-2) unstable; urgency=medium

  * Drop unnecessary dependency on python3-six. Closes: #1039011
  * Drop dependency on cython3-dbg. Closes: #1040544

 -- Jelmer Vernooĳ <jelmer@debian.org>  Sat, 24 Jun 2023 14:58:57 +0100

# Oh, and here is a comment
"#;
    let parsed = parse(CHANGELOG);
    assert_eq!(parsed.errors, Vec::<String>::new());
    let node = parsed.syntax();
    assert_eq!(
        format!("{:#?}", node),
        r###"ROOT@0..405
  ENTRY@0..140
    ENTRY_HEADER@0..39
      IDENTIFIER@0..6 "breezy"
      WHITESPACE@6..7 " "
      VERSION@7..16 "(3.3.4-1)"
      DISTRIBUTIONS@16..25
        WHITESPACE@16..17 " "
        IDENTIFIER@17..25 "unstable"
      METADATA@25..38
        SEMICOLON@25..26 ";"
        WHITESPACE@26..27 " "
        METADATA_ENTRY@27..38
          METADATA_KEY@27..34
            IDENTIFIER@27..34 "urgency"
          EQUALS@34..35 "="
          METADATA_VALUE@35..38
            IDENTIFIER@35..38 "low"
      NEWLINE@38..39 "\n"
    EMPTY_LINE@39..40
      NEWLINE@39..40 "\n"
    ENTRY_BODY@40..66
      INDENT@40..42 "  "
      DETAIL@42..65 "* New upstream release."
      NEWLINE@65..66 "\n"
    EMPTY_LINE@66..67
      NEWLINE@66..67 "\n"
    ENTRY_FOOTER@67..140
      INDENT@67..71 " -- "
      MAINTAINER@71..86
        TEXT@71..77 "Jelmer"
        WHITESPACE@77..78 " "
        TEXT@78..86 "Vernooĳ"
      WHITESPACE@86..87 " "
      EMAIL@87..106 "<jelmer@debian.org>"
      WHITESPACE@106..108 "  "
      TIMESTAMP@108..139
        TEXT@108..112 "Mon,"
        WHITESPACE@112..113 " "
        TEXT@113..115 "04"
        WHITESPACE@115..116 " "
        TEXT@116..119 "Sep"
        WHITESPACE@119..120 " "
        TEXT@120..124 "2023"
        WHITESPACE@124..125 " "
        TEXT@125..133 "18:13:45"
        WHITESPACE@133..134 " "
        TEXT@134..139 "-0500"
      NEWLINE@139..140 "\n"
  EMPTY_LINE@140..141
    NEWLINE@140..141 "\n"
  ENTRY@141..376
    ENTRY_HEADER@141..183
      IDENTIFIER@141..147 "breezy"
      WHITESPACE@147..148 " "
      VERSION@148..157 "(3.3.3-2)"
      DISTRIBUTIONS@157..166
        WHITESPACE@157..158 " "
        IDENTIFIER@158..166 "unstable"
      METADATA@166..182
        SEMICOLON@166..167 ";"
        WHITESPACE@167..168 " "
        METADATA_ENTRY@168..182
          METADATA_KEY@168..175
            IDENTIFIER@168..175 "urgency"
          EQUALS@175..176 "="
          METADATA_VALUE@176..182
            IDENTIFIER@176..182 "medium"
      NEWLINE@182..183 "\n"
    EMPTY_LINE@183..184
      NEWLINE@183..184 "\n"
    ENTRY_BODY@184..249
      INDENT@184..186 "  "
      DETAIL@186..248 "* Drop unnecessary de ..."
      NEWLINE@248..249 "\n"
    ENTRY_BODY@249..302
      INDENT@249..251 "  "
      DETAIL@251..301 "* Drop dependency on  ..."
      NEWLINE@301..302 "\n"
    EMPTY_LINE@302..303
      NEWLINE@302..303 "\n"
    ENTRY_FOOTER@303..376
      INDENT@303..307 " -- "
      MAINTAINER@307..322
        TEXT@307..313 "Jelmer"
        WHITESPACE@313..314 " "
        TEXT@314..322 "Vernooĳ"
      WHITESPACE@322..323 " "
      EMAIL@323..342 "<jelmer@debian.org>"
      WHITESPACE@342..344 "  "
      TIMESTAMP@344..375
        TEXT@344..348 "Sat,"
        WHITESPACE@348..349 " "
        TEXT@349..351 "24"
        WHITESPACE@351..352 " "
        TEXT@352..355 "Jun"
        WHITESPACE@355..356 " "
        TEXT@356..360 "2023"
        WHITESPACE@360..361 " "
        TEXT@361..369 "14:58:57"
        WHITESPACE@369..370 " "
        TEXT@370..375 "+0100"
      NEWLINE@375..376 "\n"
  EMPTY_LINE@376..377
    NEWLINE@376..377 "\n"
  COMMENT@377..405 "# Oh, and here is a c ..."
"###
    );

    let root = parsed.root();
    let entries: Vec<_> = root.entries().collect();
    assert_eq!(entries.len(), 2);
    let entry = &entries[0];
    assert_eq!(entry.package(), Some("breezy".into()));
    assert_eq!(entry.version(), Some("3.3.4-1".parse().unwrap()));
    assert_eq!(entry.distributions(), Some(vec!["unstable".into()]));
    assert_eq!(entry.urgency(), Some(Urgency::Low));
    assert_eq!(entry.maintainer(), Some("Jelmer Vernooĳ".into()));
    assert_eq!(entry.email(), Some("jelmer@debian.org".into()));
    assert_eq!(
        entry.timestamp(),
        Some("Mon, 04 Sep 2023 18:13:45 -0500".into())
    );
    assert_eq!(
        entry.datetime(),
        Some("2023-09-04T18:13:45-05:00".parse().unwrap())
    );
    let changes_lines: Vec<_> = entry.change_lines().collect();
    assert_eq!(
        changes_lines,
        vec![
            "".to_string(),
            "* New upstream release.".to_string(),
            "".to_string()
        ]
    );

    assert_eq!(node.text(), CHANGELOG);
}

#[test]
fn test_from_io_read() {
    let changelog = r#"breezy (3.3.4-1) unstable; urgency=low

  * New upstream release.

 -- Jelmer Vernooĳ <jelmer@debian.org> Mon, 04 Sep 2023 18:13:45 -0500
"#;

    let input = changelog.as_bytes();
    let input = Box::new(std::io::Cursor::new(input)) as Box<dyn std::io::Read>;
    let parsed = ChangeLog::read(input).unwrap();
    assert_eq!(parsed.to_string(), changelog);
}
