//! Files embedded in the program: `embed::bytes(path)` and
//! `embed::text(path)`, read when the program is compiled. The file is read
//! here, relative to the module that names it and inside its package, and
//! kept as a constant table of its own, which the call reads where the
//! program keeps it. It is written where a constant's value is worked out —
//! a top-level `val`, a top-level `assert` — since its contents are a
//! constant of the program.

use std::path::{Component, Path};
use std::sync::Arc;

use super::*;

impl Lowerer<'_> {
    /// A call of `embed::bytes` or, where `text`, `embed::text`: a read of
    /// the constant that holds the file.
    pub(super) fn embed(&mut self, text: bool, item: ItemUse<'_>) -> ExprId {
        let span = item.span;
        let what = if text { "embed::text" } else { "embed::bytes" };
        let args = item.args.unwrap_or(&[]);
        // Where a constant's value is worked out, and nowhere else.
        if self.const_stack.is_empty() && !self.checking_assert {
            self.embed_error(
                format!("`{what}` reads a file while the program is compiled, so it belongs in a constant"),
                span,
                "not in a constant",
                Some("write it as a constant's value, `val DATA = embed::bytes(\"file\")`, and use the constant here"),
            );
            return self.error_expr(span);
        }
        let path = match args {
            [arg] => match &self.ast.exprs[*arg].kind {
                ast::ExprKind::Str(sym) => Some(*sym),
                _ => None,
            },
            _ => None,
        };
        let Some(name) = path else {
            self.embed_error(
                format!("`{what}` takes the file's path, written as a string literal"),
                span,
                "not one string literal",
                None,
            );
            return self.error_expr(span);
        };
        let written = self.text(name).to_string();
        let module = &self.modules[self.current];
        let (Some(dir), Some(root)) = (module.dir.clone(), module.root.clone()) else {
            self.embed_error(
                format!("`{what}` has no directory to read from here"),
                span,
                "a module of the standard library, or a file checked on its own",
                None,
            );
            return self.error_expr(span);
        };
        let file = match self.embedded_file(&written, &dir, &root) {
            Ok(file) => file,
            Err((message, label)) => {
                self.embed_error(message, span, &label, None);
                return self.error_expr(span);
            }
        };
        let ty = if text {
            Types::STR
        } else {
            let bytes = self.intern(TyKind::Slice(Types::U8));
            self.intern(TyKind::Ref(bytes, crate::RefKind::Shared))
        };
        let id = match self.embedded.get(&(file.clone(), text)) {
            Some(&id) => id,
            None => {
                let bytes = match std::fs::read(&file) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        let shown = file.display();
                        self.embed_error(
                            format!("cannot read `{shown}`: {err}"),
                            span,
                            "not read",
                            None,
                        );
                        return self.error_expr(span);
                    }
                };
                if text && let Err(bad) = std::str::from_utf8(&bytes) {
                    let at = bad.valid_up_to();
                    self.embed_error(
                        format!("`{written}` is not UTF-8 text: byte {at} begins no character"),
                        span,
                        "not text",
                        Some("read it with `embed::bytes`, which takes any bytes"),
                    );
                    return self.error_expr(span);
                }
                let id = self.program.consts.alloc(ConstDef {
                    name,
                    ty,
                    value: Some(ConstValue::Bytes(Arc::from(bytes))),
                    code: None,
                    module: self.current as u32,
                    is_pub: false,
                    span,
                });
                self.embedded.insert((file, text), id);
                id
            }
        };
        let reference = self.intern(TyKind::Ref(ty, crate::RefKind::Shared));
        let address = self.alloc(ExprKind::Table(id), reference, span);
        self.alloc(ExprKind::Deref(address), ty, span)
    }

    /// Where a path a module wrote leads: a file below the module's
    /// directory, or beside it, that does not leave the package's root.
    fn embedded_file(
        &self,
        written: &str,
        dir: &Path,
        root: &Path,
    ) -> Result<std::path::PathBuf, (String, String)> {
        let relative = Path::new(written);
        if relative.has_root()
            || relative
                .components()
                .any(|part| matches!(part, Component::Prefix(_)))
        {
            return Err((
                format!(
                    "`{written}` is a whole path; a file embedded is named from the module's directory"
                ),
                "not relative to the module".to_string(),
            ));
        }
        let joined = dir.join(relative);
        let Ok(file) = joined.canonicalize() else {
            return Err((
                format!("there is no file `{written}` beside the module"),
                format!("looked for `{}`", joined.display()),
            ));
        };
        // An empty root is the current directory, as a bare file name's
        // parent is; one that cannot be found holds nothing, rather than
        // everything, as the empty path would.
        let written_root = match root.as_os_str().is_empty() {
            true => Path::new("."),
            false => root,
        };
        let Ok(root) = written_root.canonicalize() else {
            return Err((
                format!("the package's root, `{}`, cannot be found", root.display()),
                "nothing is embedded outside it".to_string(),
            ));
        };
        if !file.starts_with(&root) {
            return Err((
                format!("`{written}` is outside the package, which a file embedded may not leave"),
                format!("outside `{}`", written_root.display()),
            ));
        }
        if !file.is_file() {
            return Err((
                format!("`{written}` is not a file"),
                "a directory, or something else".to_string(),
            ));
        }
        Ok(file)
    }

    fn embed_error(&mut self, message: String, span: Span, label: &str, help: Option<&str>) {
        let mut diagnostic = Diagnostic::error(codes::EMBED, message, span, label.to_string())
            .with_note("a file embedded is read when the program is compiled, from beside the module that names it, and is a constant of the program");
        if let Some(help) = help {
            diagnostic = diagnostic.with_help(help.to_string());
        }
        self.report(diagnostic);
    }
}
