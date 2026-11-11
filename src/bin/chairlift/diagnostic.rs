//! What the CLI tells the user on stderr: per-file diagnostics, rendered by
//! `miette` with a snippet of the file where there is a position, and the
//! lines that `--verbose` adds.

use std::fmt;
use std::io::IsTerminal as _;

use miette::GraphicalReportHandler;
use miette::GraphicalTheme;
use miette::LabeledSpan;
use miette::NamedSource;
use miette::Severity;

/// Where chairlift's bugs are reported.
pub const ISSUES: &str = "https://github.com/glennib/chairlift/issues";

/// A source line longer than this is not shown in a snippet: `miette` prints
/// whole lines, and a generated file can be one line of megabytes.
const SNIPPET_LINE_LIMIT: usize = 400;

/// When to colour diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Color {
    /// On a terminal, unless `NO_COLOR` is set to a non-empty value.
    Auto,
    /// Always, also when stderr is a file or a pipe.
    Always,
    /// Never.
    Never,
}

impl Color {
    /// Whether to colour what goes to stderr.
    pub fn for_stderr(self) -> bool {
        match self {
            Color::Always => true,
            Color::Never => false,
            Color::Auto => {
                std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
                    && std::io::stderr().is_terminal()
            }
        }
    }
}

/// How much to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    /// Errors only, besides the result itself.
    Quiet,
    /// Errors, warnings, notes and the summary line.
    Normal,
    /// Also what happened to each file.
    Verbose,
}

/// Renders diagnostics and decides what is printed.
#[derive(Clone, Copy, Debug)]
pub struct Reporter {
    pub verbosity: Verbosity,
    pub color: bool,
}

impl Reporter {
    /// `diagnostic` rendered for stderr, or `None` if the verbosity leaves it
    /// out. The text ends in a newline.
    pub fn render(self, diagnostic: &Diagnostic) -> Option<String> {
        if self.verbosity == Verbosity::Quiet && diagnostic.severity != Severity::Error {
            return None;
        }
        let theme = if self.color {
            GraphicalTheme::unicode()
        } else {
            GraphicalTheme::unicode_nocolor()
        };
        // Unwrapped, so that a message stays on one line for grep and a URL
        // stays whole; the terminal wraps.
        let handler = GraphicalReportHandler::new_themed(theme).with_wrap_lines(false);
        let mut out = String::new();
        if handler.render_report(&mut out, diagnostic).is_err() {
            // Rendering into a `String` fails only if a `Display` impl does;
            // the plain message still says what happened.
            out = format!("{}\n", diagnostic.message);
        }
        Some(out)
    }
}

/// One thing to tell the user about a file.
#[derive(Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    /// What kind of problem it is, such as `chairlift::lex`.
    pub code: &'static str,
    /// The headline, starting with the file's name.
    pub message: String,
    /// The file's text, with the place the diagnostic is about.
    pub snippet: Option<Snippet>,
    pub help: Option<String>,
}

/// A labelled place in a file's text.
#[derive(Debug)]
pub struct Snippet {
    source: NamedSource<String>,
    label: LabeledSpan,
}

impl Diagnostic {
    /// An error.
    pub fn error(code: &'static str, message: String) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code,
            message,
            snippet: None,
            help: None,
        }
    }

    /// This diagnostic with `help`.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help = Some(help.into());
        self
    }

    /// This diagnostic with a snippet of `text`, the file called `name`,
    /// labelling the `char` at byte `offset` with `label`. Without a snippet
    /// when the line is too long to show.
    #[must_use]
    pub fn with_snippet(mut self, name: &str, text: String, offset: usize, label: &str) -> Self {
        let offset = text.floor_char_boundary(offset.min(text.len()));
        let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
        if line_end - line_start > SNIPPET_LINE_LIMIT {
            return self;
        }
        let len = text[offset..].chars().next().map_or(0, char::len_utf8);
        self.snippet = Some(Snippet {
            source: NamedSource::new(name, text),
            label: LabeledSpan::new_primary_with_span(Some(label.to_owned()), (offset, len)),
        });
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}

impl miette::Diagnostic for Diagnostic {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(self.code))
    }

    fn severity(&self) -> Option<Severity> {
        Some(self.severity)
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.help
            .as_ref()
            .map(|h| Box::new(h) as Box<dyn fmt::Display>)
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        self.snippet
            .as_ref()
            .map(|s| &s.source as &dyn miette::SourceCode)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        self.snippet
            .as_ref()
            .map(|s| Box::new(std::iter::once(s.label.clone())) as Box<dyn Iterator<Item = _>>)
    }
}

/// The help for a failure that only a bug in chairlift can cause.
pub fn bug_report_help() -> String {
    format!(
        "this is a bug in chairlift. Please report it at {ISSUES} with the file, or the \
         smallest part of it that shows the problem, and the output of `chairlift --version`."
    )
}
