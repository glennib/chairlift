//! Globs that exclude files and directories from formatting, as `--exclude`
//! gives them.

use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use ignore::DirEntry;
use ignore::gitignore::Gitignore;
use ignore::gitignore::GitignoreBuilder;

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
    /// `root` is absolute.
    pub fn new(root: &Path, globs: &[String]) -> Result<Self, ignore::Error> {
        let root = normalize(root);
        let mut builder = GitignoreBuilder::new(&root);
        // As `ignore::overrides` does: a `[` without its `]` is more likely
        // a mistake than a literal.
        builder.allow_unclosed_class(false);
        for glob in globs {
            builder.add_line(None, glob)?;
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
