//! `chl.toml`: the settings of the style and the excludes, as the flags
//! give them, kept in a file for the directory tree below it.

use std::fs;
use std::io;
use std::num::NonZeroU16;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use crate::Jinja;
use crate::diagnostic::Diagnostic;
use crate::exclude::Excludes;
use crate::exclude::resolve;

/// The name of the file.
const CONFIG_FILE: &str = "chl.toml";

const CODE: &str = "chairlift::config";

/// A `chl.toml`, read. A setting it leaves out is `None`.
#[derive(Debug)]
pub struct Config {
    pub path: PathBuf,
    pub line_width: Option<u16>,
    pub jinja: Option<Jinja>,
    /// Its `exclude` globs, relative to its directory; `None` when it has
    /// none.
    pub excludes: Option<Excludes>,
}

/// The keys of the file, which are the names of the flags.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Keys {
    /// Non-zero, as `--line-width` requires.
    line_width: Option<NonZeroU16>,
    jinja: Option<Jinja>,
    #[serde(default)]
    exclude: Vec<String>,
}

/// The directory to [`discover`] `chl.toml` from, for a run on `paths`,
/// which are stdin when they are `-`, with `stdin_filename` as the name it
/// gives the text: the deepest directory that holds all of `paths`, in
/// which a file counts as its directory; the directory of `stdin_filename`;
/// or, for stdin without one, the current directory. Symbolic links are
/// resolved first, as for `.chlignore`, and a directory that does not exist
/// gives way to the nearest one above it that does. `None` without a
/// current directory to make relative paths absolute.
pub fn search_from(paths: &[PathBuf], stdin_filename: Option<&Path>) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = if paths.iter().any(|p| p.as_os_str() == "-") {
        match stdin_filename {
            Some(filename) => vec![directory_of(&resolve(filename))],
            None => return std::env::current_dir().ok(),
        }
    } else {
        paths
            .iter()
            .map(|path| {
                let path = resolve(path);
                if path.is_dir() {
                    path
                } else {
                    directory_of(&path)
                }
            })
            .collect()
    };
    let common = dirs.into_iter().reduce(|a, b| {
        a.components()
            .zip(b.components())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a)
            .collect()
    })?;
    // Paths on two Windows drives have no directory in common.
    if !common.is_absolute() {
        return std::env::current_dir().ok();
    }
    common
        .ancestors()
        .find(|dir| dir.is_dir())
        .map(Path::to_path_buf)
}

/// The directory that holds the file at `path`; `path` itself for a root.
fn directory_of(path: &Path) -> PathBuf {
    path.parent().unwrap_or(path).to_path_buf()
}

/// The `chl.toml` in `dir` or the nearest directory above it that has one,
/// read; `None` when none has. `dir` is absolute.
pub fn discover(dir: &Path) -> Result<Option<Config>, Diagnostic> {
    for dir in dir.ancestors() {
        let path = dir.join(CONFIG_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => return parse(dir, path, text).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(Diagnostic::error(
                    CODE,
                    format!("{}: cannot be read: {e}", path.display()),
                ));
            }
        }
    }
    Ok(None)
}

/// The `chl.toml` at `path`, in `dir`, whose text is `text`.
fn parse(dir: &Path, path: PathBuf, text: String) -> Result<Config, Diagnostic> {
    let name = path.display().to_string();
    let keys: Keys = match toml::from_str(&text) {
        Ok(keys) => keys,
        Err(e) => {
            let message = e.message().trim_end();
            return Err(match e.span() {
                Some(span) => {
                    let at = chairlift::Position::new(&text, span.start);
                    Diagnostic::error(CODE, format!("{name}:{at}: {message}"))
                        .with_snippet(&name, text, span.start, "here")
                }
                None => Diagnostic::error(CODE, format!("{name}: {message}")),
            });
        }
    };
    let excludes = if keys.exclude.is_empty() {
        None
    } else {
        let excludes = Excludes::new(dir, Some(&path), &keys.exclude)
            .map_err(|e| Diagnostic::error(CODE, format!("{name}: invalid exclude: {e}")))?;
        Some(excludes)
    };
    Ok(Config {
        line_width: keys.line_width.map(NonZeroU16::get),
        jinja: keys.jinja,
        excludes,
        path,
    })
}
