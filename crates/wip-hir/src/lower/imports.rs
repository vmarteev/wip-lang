//! Imports and visibility: what a file's `import`s bind, and which items
//! of other modules it may name.

use super::*;

/// A function that `files` declare under `sym`: its name, whether it is
/// exported, and whether it is a C function. A C declaration is exported
/// when it says `pub`, so that bindings are a module people share.
fn declared_fn(files: &[&Ast], sym: Symbol) -> Option<(ast::Name, bool, bool)> {
    files
        .iter()
        .flat_map(|file| &file.items)
        .find_map(|item| match item {
            ast::Item::Fn(f) if f.sig.name.sym == sym => Some((f.sig.name, f.is_pub, false)),
            ast::Item::Extern(e) => e
                .fns
                .iter()
                .find(|declared| declared.sig.name.sym == sym)
                .map(|declared| (declared.sig.name, declared.is_pub, true)),
            _ => None,
        })
}

/// A top-level `val` of these files, and whether it is exported: read from
/// the files, since a module's constants are declared after the imports
/// that name them are checked.
fn declared_val(files: &[&Ast], sym: Symbol) -> Option<(ast::Name, bool)> {
    files
        .iter()
        .flat_map(|file| &file.items)
        .find_map(|item| match item {
            ast::Item::Val(v) if v.name.sym == sym => Some((v.name, v.is_pub)),
            _ => None,
        })
}

/// A variable C owns, declared in an `extern "C"` block of these files, and
/// whether it is exported: a `pub val` or `pub var` is imported by name as a
/// function or a constant is.
fn declared_global(files: &[&Ast], sym: Symbol) -> Option<(ast::Name, bool)> {
    files
        .iter()
        .flat_map(|file| &file.items)
        .find_map(|item| match item {
            ast::Item::Extern(e) => e
                .globals
                .iter()
                .find(|declared| declared.name.sym == sym)
                .map(|declared| (declared.name, declared.is_pub)),
            _ => None,
        })
}

impl<'a> Lowerer<'a> {
    /// The module a path written in the current module means: one of
    /// `std`'s, one of a dependency's — whose name begins the path — or
    /// one of this package's own, named from the package's root.
    pub(super) fn module_meant(&self, written: &str) -> String {
        let scope = &self.modules[self.current];
        let first = written.split("::").next().unwrap_or("");
        if scope.prefix.is_empty() || first == "std" || scope.depends.iter().any(|d| d == first) {
            written.to_string()
        } else if written.is_empty() {
            scope.prefix.clone()
        } else {
            format!("{}::{written}", scope.prefix)
        }
    }

    /// What the current file's `import` declarations bind: modules, and the
    /// items named in braces. The items are checked by
    /// [`Lowerer::check_imported_items`], once every type is named.
    pub(super) fn resolve_imports(&mut self) -> FileImports {
        let ast = self.ast;
        let mut imports = FileImports::default();
        let mut first_seen: FxHashMap<Symbol, Span> = FxHashMap::default();
        for item in &ast.items {
            let ast::Item::Import(decl) = item else {
                continue;
            };
            let written: Vec<&str> = decl.path.iter().map(|n| self.text(n.sym)).collect();
            let written = written.join("::");
            let meant = self.module_meant(&written);
            let index = self.modules.iter().position(|p| p.path == meant);
            match index {
                None => {
                    let diagnostic = Diagnostic::error(
                        codes::UNKNOWN_MODULE,
                        format!("cannot find module `{written}`"),
                        decl.span,
                        "unknown module",
                    )
                    .with_note("a module is a directory below the one the program starts in, named by its path");
                    self.report(diagnostic);
                }
                Some(index) if index == self.current => {
                    let diagnostic = Diagnostic::error(
                        codes::UNKNOWN_MODULE,
                        "a module cannot import itself",
                        decl.span,
                        "this is the module being compiled",
                    )
                    .with_note("the files of one module share a namespace already");
                    self.report(diagnostic);
                }
                Some(_) => {}
            }
            let index = index.filter(|&index| index != self.current);
            let last = *decl.path.last().expect("a path has segments");
            // What the import binds, and to what: the module, or the names
            // in its braces, where `self` is the module.
            let bindings: Vec<(ast::Name, Option<ast::Name>)> = match &decl.items {
                None => vec![(decl.alias.unwrap_or(last), None)],
                Some(items) => items
                    .iter()
                    .map(|item| match item.name {
                        // `self` binds the module under its last segment.
                        None => {
                            let own = ast::Name {
                                sym: last.sym,
                                span: item.span,
                            };
                            (item.alias.unwrap_or(own), None)
                        }
                        Some(name) => (item.alias.unwrap_or(name), Some(name)),
                    })
                    .collect(),
            };
            for (bound, item) in bindings {
                if let Some(&first) = first_seen.get(&bound.sym) {
                    let name = self.text(bound.sym).to_string();
                    let diagnostic = Diagnostic::error(
                        codes::DUPLICATE_DEFINITION,
                        format!("`{name}` is imported twice"),
                        bound.span,
                        "already bound by another import",
                    )
                    .with_secondary(first, "the first import")
                    .with_help("give one of them another name with `as`");
                    self.report(diagnostic);
                    continue;
                }
                first_seen.insert(bound.sym, bound.span);
                match (index, item) {
                    (Some(index), None) => {
                        imports.modules.insert(bound.sym, index);
                    }
                    (Some(index), Some(item)) => {
                        imports
                            .items
                            .insert(bound.sym, ImportedItem::Item(index, item));
                    }
                    // The module was reported; uses of its items stay quiet.
                    (None, Some(_)) => {
                        imports.items.insert(bound.sym, ImportedItem::Broken);
                    }
                    (None, None) => {}
                }
            }
        }
        imports
    }

    /// Checks the items the current file imports by name.
    /// Each must be declared and exported by its module, and must not share
    /// a name with something the importing module declares. Returns the
    /// file's imports with every item reported here marked broken, so that
    /// its uses stay quiet.
    pub(super) fn check_imported_items(&mut self, modules: &[ModuleAst<'a>]) -> FileImports {
        let mut imports = std::mem::take(&mut self.imports);
        let ast = self.ast;
        for item in &ast.items {
            let ast::Item::Import(decl) = item else {
                continue;
            };
            // A name of the prelude is in scope already. The
            // module's own name is bound where the import has no list.
            let module_name = decl.alias.or_else(|| match decl.items {
                None => decl.path.last().copied(),
                Some(_) => None,
            });
            if let Some(bound) = module_name
                && self.prelude_name(bound, "an import named")
            {
                imports.modules.remove(&bound.sym);
            }
            for item in decl.items.iter().flatten() {
                let Some(name) = item.name else { continue };
                let bound = item.alias.unwrap_or(name);
                if self.prelude_name(bound, "an import named") {
                    imports.items.insert(bound.sym, ImportedItem::Broken);
                    continue;
                }
                // A name imported twice was reported, and keeps its first
                // import.
                let Some(&ImportedItem::Item(module, name)) = imports.items.get(&bound.sym) else {
                    continue;
                };
                if name.span != item.span {
                    continue;
                }
                if !self.check_imported_item(modules, module, name, bound) {
                    imports.items.insert(bound.sym, ImportedItem::Broken);
                }
            }
        }
        imports
    }

    /// Reports what is wrong with importing `name` from `module` under the
    /// name `bound`, and returns whether nothing is.
    pub(super) fn check_imported_item(
        &mut self,
        modules: &[ModuleAst<'a>],
        module: usize,
        name: ast::Name,
        bound: ast::Name,
    ) -> bool {
        let text = self.text(name.sym).to_string();
        let path = self.modules[module].path.clone();
        let ty = self.modules[module]
            .types
            .get(&name.sym)
            .map(|&(def, _)| match def {
                TypeDef::Struct(id) => ("struct", self.program.structs[id].is_pub),
                TypeDef::Enum(id) => ("enum", self.program.enums[id].is_pub),
                TypeDef::Builtin(_) => ("type", false),
            });
        // A type alias, which `pub` exports as any item,
        // and a C type, seen only through `ptr`.
        let ty = ty
            .or_else(|| {
                self.modules[module]
                    .aliases
                    .get(&name.sym)
                    .map(|alias| ("type alias", alias.is_pub))
            })
            .or_else(|| {
                self.modules[module]
                    .opaques
                    .get(&name.sym)
                    .map(|&id| ("C type", self.program.opaques[id].is_pub))
            });
        let func = declared_fn(&modules[module].files, name.sym);
        // A top-level `val` is an item too.
        let konst = declared_val(&modules[module].files, name.sym).map(|(_, is_pub)| is_pub);
        let global = declared_global(&modules[module].files, name.sym).map(|(_, is_pub)| is_pub);
        let exported = ty.is_some_and(|(_, is_pub)| is_pub)
            || func.is_some_and(|(_, is_pub, _)| is_pub)
            || konst == Some(true)
            || global == Some(true);
        if !exported {
            match (ty, func) {
                (None, None) if konst == Some(false) => self.private_item(module, "val", name),
                (None, None) if global == Some(false) => self.private_item(module, "var", name),
                (Some((what, _)), _) => self.private_item(module, what, name),
                (None, Some((_, _, false))) => self.private_item(module, "fn", name),
                (None, Some((_, _, true))) => {
                    let diagnostic = Diagnostic::error(
                        codes::PRIVATE_ITEM,
                        format!("`{text}` is a C function that module `{path}` declares"),
                        name.span,
                        "an extern declaration",
                    )
                    .with_help(format!(
                        "declare `fn {text}` in an `extern \"C\"` block of this module"
                    ))
                    .with_note("extern declarations are never exported");
                    self.report(diagnostic);
                }
                (None, None) => {
                    let mut candidates: Vec<&str> = self.modules[module]
                        .types
                        .iter()
                        .filter(|&(_, &(def, _))| match def {
                            TypeDef::Struct(id) => self.program.structs[id].is_pub,
                            TypeDef::Enum(id) => self.program.enums[id].is_pub,
                            TypeDef::Builtin(_) => false,
                        })
                        .map(|(&sym, _)| self.text(sym))
                        .collect();
                    candidates.extend(
                        modules[module]
                            .files
                            .iter()
                            .flat_map(|file| &file.items)
                            .filter_map(|item| match item {
                                ast::Item::Fn(f) if f.is_pub => Some(self.text(f.sig.name.sym)),
                                ast::Item::Val(v) if v.is_pub => Some(self.text(v.name.sym)),
                                _ => None,
                            }),
                    );
                    let mut diagnostic = Diagnostic::error(
                        codes::UNKNOWN_NAME,
                        format!("cannot find `{text}` in module `{path}`"),
                        name.span,
                        "not declared there",
                    );
                    if let Some(similar) = suggest(&text, candidates) {
                        diagnostic = diagnostic.with_fix(
                            format!("did you mean `{similar}`?"),
                            [Edit::replace(name.span, similar)],
                        );
                    }
                    self.report(diagnostic);
                }
            }
            return false;
        }
        // The module's own names come first, so the import would never be
        // used.
        let own_type = self.types().get(&bound.sym).map(|&(_, span)| span);
        let own_fn = declared_fn(&modules[self.current].files, bound.sym)
            .map(|(declared, ..)| declared.span);
        let own_const = declared_val(&modules[self.current].files, bound.sym)
            .map(|(declared, _)| declared.span);
        if let Some(declared) = own_type.or(own_fn).or(own_const) {
            let bound_text = self.text(bound.sym).to_string();
            let diagnostic = Diagnostic::error(
                codes::DUPLICATE_DEFINITION,
                format!("`{bound_text}` is imported, and declared in this module too"),
                bound.span,
                "imported here",
            )
            .with_secondary(declared, "declared here")
            .with_help(format!("import it under another name: `{text} as …`"));
            self.report(diagnostic);
            return false;
        }
        true
    }

    /// What `name` stands for if the current file imported it from another
    /// module: the module, and the item's name there,
    /// spanned where it is used. `None` if nothing was imported under the
    /// name, or the current module declares the name itself.
    pub(super) fn imported(&self, name: ast::Name) -> Option<ImportedItem> {
        if self.types().contains_key(&name.sym)
            || self.fns().contains_key(&name.sym)
            || self.consts().contains_key(&name.sym)
        {
            return None;
        }
        Some(match *self.imports.items.get(&name.sym)? {
            ImportedItem::Item(module, item) => ImportedItem::Item(
                module,
                ast::Name {
                    sym: item.sym,
                    span: name.span,
                },
            ),
            ImportedItem::Broken => ImportedItem::Broken,
        })
    }

    /// Whether an item of `module` may be named here: a module sees all of
    /// its own declarations, and only what other modules export.
    pub(super) fn visible(&self, module: usize, is_pub: bool) -> bool {
        module == self.current || is_pub
    }

    /// Reports an item that another module keeps to itself.
    pub(super) fn private_item(&mut self, module: usize, what: &str, name: ast::Name) {
        let text = self.text(name.sym).to_string();
        let path = self.modules[module].path.clone();
        // A C type is declared `type name` inside an extern block, and an
        // alias `type name = …`, so that is what the help says to write.
        let declared = match what {
            "C type" | "type alias" => "type".to_string(),
            what => what.to_string(),
        };
        let diagnostic = Diagnostic::error(
            codes::PRIVATE_ITEM,
            format!("`{text}` is private to module `{path}`"),
            name.span,
            format!("a private {what}"),
        )
        .with_help(format!("declare it `pub {declared} {text}` to export it"));
        self.report(diagnostic);
    }
}
