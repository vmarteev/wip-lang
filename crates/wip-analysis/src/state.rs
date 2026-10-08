//! The move state: which places are moved out of, and how states merge where
//! control flow joins.

use super::*;

impl Path {
    /// Whether `self` is `other` or lies inside it.
    pub(super) fn within(&self, other: &Path) -> bool {
        self.local == other.local && self.projs.starts_with(&other.projs)
    }

    /// Whether it stands for what a parameter borrows, which nothing the
    /// call does changes.
    pub(super) fn is_lent(&self) -> bool {
        self.projs.first() == Some(&Proj::Lent)
    }

    /// Whether a root and a place overlap, so that changing the place
    /// changes what borrows the root.
    pub(super) fn overlaps(&self, place: &Path) -> bool {
        !self.is_lent() && (self.within(place) || place.within(self))
    }
}

impl State {
    /// A moved place that overlaps `path`: one that `path` lies inside, or one
    /// inside `path`.
    pub(super) fn conflict(&self, path: &Path) -> Option<(&Path, &Moved)> {
        self.moved
            .iter()
            .find(|(moved, _)| path.within(moved) || moved.within(path))
    }

    pub(super) fn record_move(&mut self, path: Path, span: Span, name: String, by_defer: bool) {
        self.moved.retain(|moved, _| !moved.within(&path));
        self.moved.insert(
            path,
            Moved {
                definite: true,
                spans: vec![span],
                name,
                by_defer,
            },
        );
    }

    /// `path` has been assigned: it, and everything inside it, holds a value.
    pub(super) fn reinit(&mut self, path: &Path) {
        self.moved.retain(|moved, _| !moved.within(path));
    }

    /// The state where control flow from `self` and `other` joins.
    pub(super) fn merge(&self, other: &State) -> State {
        let mut moved = BTreeMap::new();
        for (path, m) in &self.moved {
            let merged = match other.moved.get(path) {
                Some(o) => {
                    let mut spans = m.spans.clone();
                    spans.extend(o.spans.iter().copied());
                    spans.sort_by_key(|s| (s.lo, s.hi));
                    spans.dedup();
                    Moved {
                        definite: m.definite && o.definite,
                        spans,
                        name: m.name.clone(),
                        by_defer: m.by_defer || o.by_defer,
                    }
                }
                None => Moved {
                    definite: false,
                    ..m.clone()
                },
            };
            moved.insert(path.clone(), merged);
        }
        for (path, o) in &other.moved {
            if !self.moved.contains_key(path) {
                moved.insert(
                    path.clone(),
                    Moved {
                        definite: false,
                        ..o.clone()
                    },
                );
            }
        }
        // A variable borrows whatever it borrows on any path here.
        let mut roots = self.roots.clone();
        for (local, paths) in &other.roots {
            roots
                .entry(*local)
                .or_default()
                .extend(paths.iter().cloned());
        }
        // Stale on every path, or only on some, as a move is.
        let mut stale = BTreeMap::new();
        for (local, s) in &self.stale {
            let definite = s.definite && other.stale.get(local).is_some_and(|o| o.definite);
            stale.insert(
                *local,
                borrowed::Stale {
                    definite,
                    ..s.clone()
                },
            );
        }
        for (local, o) in &other.stale {
            stale.entry(*local).or_insert_with(|| borrowed::Stale {
                definite: false,
                ..o.clone()
            });
        }
        // A copy is unread where it is unread on any path.
        let mut copied = self.copied.clone();
        for (local, spans) in &other.copied {
            let merged = copied.entry(*local).or_default();
            for span in spans {
                if !merged.contains(span) {
                    merged.push(*span);
                }
            }
        }
        State {
            moved,
            roots,
            stale,
            copied,
        }
    }
}
