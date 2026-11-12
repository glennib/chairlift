//! What excludes files and directories from formatting: globs, as
//! `--exclude` gives them, and ignore files such as `.chlignore`, both in
//! the walk of a directory and for a path matched on its own.

use std::collections::HashMap;
use std::fmt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use ignore::DirEntry;
use ignore::Match;
use ignore::gitignore::Gitignore;
use ignore::gitignore::GitignoreBuilder;
use ignore::gitignore::Glob;

/// Globs in gitignore syntax, matched relative to a root directory, with
/// the meaning they would have as the lines of a `.gitignore` there: a
/// later `!glob` brings back what an earlier glob excluded.
///
/// A path outside the root is matched as an absolute path, so only a glob
/// without a slash before its end, which matches at any depth, can match
/// it.
#[derive(Debug)]
pub struct Excludes {
    root: PathBuf,
    /// `root` with symbolic links resolved, for a path that reaches the root
    /// through one.
    canonical_root: Option<PathBuf>,
    globs: Gitignore,
}

impl Excludes {
    /// `root` is absolute. `from` is the file the globs are read from, if
    /// they are, which an [`Exclusion`] names.
    pub fn new(root: &Path, from: Option<&Path>, globs: &[String]) -> Result<Self, ignore::Error> {
        let root = normalize(root);
        let mut builder = GitignoreBuilder::new(&root);
        // As `ignore::overrides` does: a `[` without its `]` is more likely
        // a mistake than a literal.
        builder.allow_unclosed_class(false);
        for glob in globs {
            builder.add_line(from.map(Path::to_path_buf), glob)?;
        }
        Ok(Self {
            canonical_root: root.canonicalize().ok(),
            globs: builder.build()?,
            root,
        })
    }

    /// A predicate for [`ignore::WalkBuilder::filter_entry`] on the walk of
    /// `dir`, false for an entry a glob excludes. The walk does not go into
    /// an excluded directory, so an entry's parents are not matched again;
    /// nor is `dir`, which the walk never filters.
    pub fn walk_filter(
        self: &Arc<Self>,
        dir: &Path,
    ) -> impl Fn(&DirEntry) -> bool + Send + Sync + 'static {
        let excludes = Arc::clone(self);
        let dir = dir.to_path_buf();
        let located = self.locate(&dir);
        move |entry| {
            let below = entry.path().strip_prefix(&dir).unwrap_or(entry.path());
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            !excludes
                .globs
                .matched(located.join(below), is_dir)
                .is_ignore()
        }
    }

    /// The glob that excludes `path`, relative to the current directory or
    /// absolute, if one does. The answer is the one a walk of a directory
    /// above `path` would give: a directory a glob excludes excludes
    /// everything beneath it, whatever a later `!glob` says of what is
    /// beneath.
    pub fn excludes(&self, path: &Path, is_dir: bool) -> Option<Exclusion> {
        let located = self.locate(path);
        top_down(&located, is_dir)
            .into_iter()
            .find_map(|(step, is_dir)| match self.globs.matched(step, is_dir) {
                Match::Ignore(glob) => Some(glob.into()),
                Match::None | Match::Whitelist(_) => None,
            })
    }

    /// `path`, relative to the current directory or absolute, as the globs
    /// see it: relative to the root, or absolute when it is outside.
    pub fn locate(&self, path: &Path) -> PathBuf {
        let absolute = normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
        if let Ok(relative) = absolute.strip_prefix(&self.root) {
            return relative.to_path_buf();
        }
        if let (Some(root), Ok(canonical)) = (&self.canonical_root, path.canonicalize())
            && let Ok(relative) = canonical.strip_prefix(root)
        {
            return relative.to_path_buf();
        }
        absolute
    }
}

/// The ignore files of one name, such as `.chlignore`, in the directories
/// above the paths matched, each read once.
#[derive(Debug)]
pub struct IgnoreFiles {
    name: &'static str,
    read: HashMap<PathBuf, Gitignore>,
    errors: Vec<ignore::Error>,
}

impl IgnoreFiles {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            read: HashMap::new(),
            errors: Vec::new(),
        }
    }

    /// The line of an ignore file that excludes `path`, relative to the
    /// current directory or absolute, if one does; `path` need not exist.
    ///
    /// The answer is the one [`ignore::WalkBuilder`] gives, with
    /// `parents(true)`, for `path` in a walk of a directory above it: the
    /// ignore file in each directory from the file system root down applies
    /// beneath that directory, with its globs relative to it; the deepest
    /// one that matches decides, so its `!glob` brings back what a shallower
    /// one excludes; and an excluded directory excludes everything beneath
    /// it, as the walk does not go into it. Symbolic links are resolved
    /// first, as the walk resolves them in its root.
    ///
    /// An ignore file that cannot be read in full is used as far as it can
    /// be, and its error kept for [`Self::take_errors`].
    pub fn excludes(&mut self, path: &Path, is_dir: bool) -> Option<Exclusion> {
        let path = resolve(path);
        for (step, is_dir) in top_down(&path, is_dir) {
            for dir in step.ancestors().skip(1) {
                match self.read(dir).matched(step, is_dir) {
                    Match::None => {}
                    Match::Whitelist(_) => break,
                    Match::Ignore(glob) => return Some(glob.into()),
                }
            }
        }
        None
    }

    /// Reads the ignore file in `dir`, unless it has been read, for the
    /// errors [`Self::take_errors`] gives.
    pub fn check(&mut self, dir: &Path) {
        self.read(&resolve(dir));
    }

    /// The errors from the ignore files read since the last call.
    pub fn take_errors(&mut self) -> Vec<ignore::Error> {
        std::mem::take(&mut self.errors)
    }

    fn read(&mut self, dir: &Path) -> &Gitignore {
        let Self { name, read, errors } = self;
        read.entry(dir.to_path_buf()).or_insert_with(|| {
            // A file that does not exist is no error.
            let (gitignore, error) = Gitignore::new(dir.join(name));
            errors.extend(error);
            gitignore
        })
    }
}

/// What excludes a path from formatting: a glob, and the ignore file it is
/// a line of, if it is one.
#[derive(Debug)]
pub struct Exclusion {
    pub glob: String,
    /// `None` for an `--exclude` glob.
    pub from: Option<PathBuf>,
}

impl From<&Glob> for Exclusion {
    fn from(glob: &Glob) -> Self {
        Self {
            glob: glob.original().to_owned(),
            from: glob.from().map(Path::to_path_buf),
        }
    }
}

impl fmt::Display for Exclusion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.from {
            Some(file) => write!(f, "`{}` in {}", self.glob, file.display()),
            None => write!(f, "--exclude `{}`", self.glob),
        }
    }
}

/// `error` without the errors of the ignore files called `name`; `None`
/// when nothing else is left.
pub fn without_errors_of(error: ignore::Error, name: &str) -> Option<ignore::Error> {
    match error {
        ignore::Error::Partial(errors) => {
            let mut rest: Vec<_> = errors
                .into_iter()
                .filter_map(|error| without_errors_of(error, name))
                .collect();
            match rest.len() {
                0 | 1 => rest.pop(),
                _ => Some(ignore::Error::Partial(rest)),
            }
        }
        ignore::Error::WithPath { ref path, .. } if path.file_name() == Some(name.as_ref()) => None,
        error => Some(error),
    }
}

/// The ancestors of `path` that have a name, as directories, then `path`,
/// in the order a walk from the top meets them; nothing for a path without
/// a name, such as a root.
fn top_down(path: &Path, is_dir: bool) -> Vec<(&Path, bool)> {
    if path.file_name().is_none() {
        return Vec::new();
    }
    let mut steps: Vec<(&Path, bool)> = path
        .ancestors()
        .skip(1)
        .filter(|ancestor| ancestor.file_name().is_some())
        .map(|ancestor| (ancestor, true))
        .collect();
    steps.reverse();
    steps.push((path, is_dir));
    steps
}

/// `path`, absolute, with symbolic links resolved in as much of it as
/// exists.
pub fn resolve(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let absolute = normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
    let mut existing = absolute.as_path();
    let mut rest = Vec::new();
    while let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) {
        rest.push(name);
        existing = parent;
        if let Ok(canonical) = existing.canonicalize() {
            return rest
                .iter()
                .rev()
                .fold(canonical, |path, name| path.join(name));
        }
    }
    absolute
}

/// `path` without `.` components, and with each `..` taking away the
/// component before it, without asking the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(normal.components().next_back(), Some(Component::Normal(_))) =>
            {
                normal.pop();
            }
            _ => normal.push(component),
        }
    }
    normal
}
