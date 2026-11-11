//! A document as indented text, for debugging and snapshots.

use std::fmt;

use super::Doc;
use super::ir::LineKind;
use super::ir::NONE;
use super::ir::Node;
use super::ir::Place;
use super::ir::Scope;
use super::ir::Sep;

/// One node per line, indented by nesting: tokens with their text and how
/// they are placed, layout whitespace, groups (with their id, and `forced`
/// when they never print flat) and scopes.
impl fmt::Debug for Doc<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut depth = 0usize;
        for node in &self.nodes {
            if matches!(node, Node::GroupEnd | Node::ScopeEnd) {
                depth = depth.saturating_sub(1);
                continue;
            }
            for _ in 0..depth {
                f.write_str("  ")?;
            }
            match *node {
                Node::Atom(atom) => {
                    let text = self.token(atom.token).text(self.source);
                    write!(f, "{} {:?}", atom.token, text)?;
                    match atom.place {
                        Place::Inline if atom.aux != NONE => {
                            write!(f, " as {:?}", self.statics[atom.aux as usize])?;
                        }
                        Place::Inline => {}
                        Place::Trailing => f.write_str(" trailing")?,
                        Place::OwnLine => f.write_str(" own-line")?,
                        Place::Tag => {
                            write!(f, " tag #{}", atom.aux)?;
                            let opener = self.tags[atom.aux as usize];
                            if opener != NONE {
                                write!(f, " aligned with #{opener}")?;
                            }
                        }
                    }
                    write_sep(f, atom.sep)?;
                }
                Node::Verbatim(v) => {
                    let v = self.verbatims[v as usize];
                    write!(f, "verbatim {}..={}", v.first, v.last)?;
                    if v.exact {
                        f.write_str(" exact")?;
                    }
                    if v.own_line {
                        f.write_str(" own-line")?;
                    }
                    write_sep(f, v.sep)?;
                }
                Node::Space => f.write_str("space")?,
                Node::Line(LineKind::Soft) => f.write_str("soft-line")?,
                Node::Line(LineKind::Normal) => f.write_str("line")?,
                Node::Line(LineKind::Hard) => f.write_str("hard-line")?,
                Node::BlankLines(n) => write!(f, "blank-lines {n}")?,
                Node::BlankIfBroken(g) => write!(f, "blank-line-if-broken #{g}")?,
                Node::IfBreak {
                    group,
                    broken,
                    flat,
                } => write!(f, "if-break #{group} {broken:?} else {flat:?}")?,
                Node::Frozen { start, end } => {
                    write!(f, "frozen {:?}", &self.source[start as usize..end as usize])?;
                }
                Node::GroupStart(g) => {
                    write!(f, "group #{g}")?;
                    if self.groups[g as usize].forced {
                        f.write_str(" forced")?;
                    }
                    depth += 1;
                }
                Node::ScopeStart(scope) => {
                    f.write_str(match scope {
                        Scope::Indent => "indent",
                        Scope::Dedent => "dedent",
                        Scope::Root => "at-root",
                        Scope::Hug => "hug",
                    })?;
                    depth += 1;
                }
                Node::GroupEnd | Node::ScopeEnd => {}
            }
            f.write_str("\n")?;
        }
        Ok(())
    }
}

fn write_sep(f: &mut fmt::Formatter<'_>, sep: Sep) -> fmt::Result {
    match sep {
        Sep::Free => Ok(()),
        Sep::Glue => f.write_str(" (spaced)"),
        Sep::Touch => f.write_str(" (touching)"),
    }
}
