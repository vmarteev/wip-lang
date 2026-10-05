//! Paths that name items: `a::b::Item`, `Enum::Variant` and
//! `pkg::Enum::Variant`.

use super::*;

/// A function or variant used in an expression: the arguments of the call
/// around it, if there is one, the type arguments written in its path, and
/// the type expected of the value.
#[derive(Clone, Copy)]
pub(super) struct ItemUse<'a> {
    pub args: Option<&'a [ast::ExprId]>,
    /// The names the arguments were given, if any.
    pub names: &'a [Option<ast::Name>],
    pub type_args: Option<&'a ast::TypeArgs>,
    /// The segment of the path that names the generic item, which type
    /// arguments must follow.
    pub name_segment: usize,
    pub hint: Option<Ty>,
    /// For a `static fn` reached through a type alias, the type the alias
    /// names, which says what the owner's parameters are.
    pub owner_ty: Option<Ty>,
    /// For a call through a constraint, the interface's own type
    /// arguments, which the constraint carries.
    pub interface_args: crate::TyList,
    /// For a method call, the receiver, already checked: it is the first
    /// parameter, and is not written among the arguments.
    pub receiver: Option<ExprId>,
    /// For a call of an interface's method through a type parameter, the
    /// type `Self` stands for.
    pub self_ty: Option<Ty>,
    pub span: Span,
}

impl<'a> ItemUse<'a> {
    /// The name each argument was given: none, where no names were kept.
    pub fn arg_names(&self) -> Vec<Option<ast::Name>> {
        let count = self.args.map_or(0, <[_]>::len);
        (0..count)
            .map(|i| self.names.get(i).copied().flatten())
            .collect()
    }

    /// A use with no type arguments written.
    pub fn plain(args: Option<&'a [ast::ExprId]>, hint: Option<Ty>, span: Span) -> ItemUse<'a> {
        ItemUse {
            args,
            names: &[],
            type_args: None,
            name_segment: 0,
            hint,
            receiver: None,
            self_ty: None,
            owner_ty: None,
            interface_args: crate::TyList::EMPTY,
            span,
        }
    }
}

/// What a path names once its module is known.
pub(super) enum PathTarget {
    /// An item of the module: a function or a type.
    Item(usize, ast::Name),
    /// A variant: the module, the enum's name and the variant's name.
    Variant(usize, ast::Name, ast::Name),
    /// A path already reported: it names no module, or starts with an
    /// import that was reported.
    Broken,
}

impl<'a> Lowerer<'a> {
    /// Splits a path into the module it names and the segments after it.
    /// A path that starts with an imported item
    /// continues in that item's module, under the item's own name, and a
    /// path that names no module belongs to the current one. `None` if the
    /// path starts with an import that was already reported.
    pub(super) fn split_path<'n>(
        &self,
        segments: &'n [ast::Name],
    ) -> Option<(usize, Cow<'n, [ast::Name]>)> {
        let first = *segments.first()?;
        if segments.len() > 1
            && let Some(&module) = self.imports.modules.get(&first.sym)
        {
            return Some((module, Cow::Borrowed(&segments[1..])));
        }
        match self.imported(first) {
            Some(ImportedItem::Item(module, item)) => {
                let mut renamed = segments.to_vec();
                renamed[0] = item;
                return Some((module, Cow::Owned(renamed)));
            }
            Some(ImportedItem::Broken) => return None,
            None => {}
        }
        for end in (1..segments.len()).rev() {
            let written: Vec<&str> = segments[..end].iter().map(|n| self.text(n.sym)).collect();
            let written = written.join("::");
            let meant = self.module_meant(&written);
            if let Some(index) = self.modules.iter().position(|p| p.path == meant) {
                return Some((index, Cow::Borrowed(&segments[end..])));
            }
        }
        Some((self.current, Cow::Borrowed(segments)))
    }

    /// The module and item a path names, where it names one, without
    /// reporting what it cannot find: a caller that goes on resolves the
    /// path again, and that reports.
    pub(super) fn resolve_path_quietly(&self, segments: &[ast::Name]) -> PathTarget {
        match self.split_path(segments) {
            Some((module, rest)) => match *rest {
                [item] => PathTarget::Item(module, item),
                _ => PathTarget::Broken,
            },
            None => PathTarget::Broken,
        }
    }

    /// Whether `module` holds a type called `sym`, and no function of that
    /// name: the prelude's, where `module` is this one.
    pub(super) fn module_types_contain(&self, module: usize, sym: Symbol) -> bool {
        let here = &self.modules[module];
        if here.fns.contains_key(&sym) {
            return false;
        }
        here.types.contains_key(&sym)
            || here.aliases.contains_key(&sym)
            || (module == self.current
                && self.prelude_module(sym).is_some()
                && self.prelude_fn(sym).is_none())
    }

    /// Resolves the module a path starts in, and what the rest of it names.
    /// `Enum::Variant` is accepted only if `variants` is set. Anything else
    /// is reported as an unknown module, with `note`.
    pub(super) fn resolve_path(
        &mut self,
        segments: &[ast::Name],
        variants: bool,
        note: &str,
    ) -> PathTarget {
        let Some((module, rest)) = self.split_path(segments) else {
            return PathTarget::Broken;
        };
        match *rest {
            [item] => PathTarget::Item(module, item),
            [enum_name, variant] if variants => PathTarget::Variant(module, enum_name, variant),
            [first, ..] => {
                self.unknown_module(first, note);
                PathTarget::Broken
            }
            [] => PathTarget::Broken,
        }
    }

    /// Reports `name` as a module that does not exist.
    pub(super) fn unknown_module(&mut self, name: ast::Name, note: &str) {
        let text = self.text(name.sym).to_string();
        let diagnostic = Diagnostic::error(
            codes::UNKNOWN_MODULE,
            format!("cannot find module `{text}`"),
            name.span,
            "unknown module",
        )
        .with_note(note);
        self.report(diagnostic);
    }
}

/// The note for an unknown module in most paths.
pub(super) const MODULE_NOTE: &str = "a module is a directory, named by its path";
