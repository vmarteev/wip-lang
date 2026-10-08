//! Top-level `assert`s: facts the program relies on, checked while it is
//! compiled. Each is checked as any `assert` is, in a body
//! of its own, which becomes a function the compiler runs once the program
//! is checked, as it runs a `@comptime` constant's; a
//! condition that does not hold is a compile error there.

use super::*;

impl Lowerer<'_> {
    /// Checks the file's top-level asserts, and keeps each as the function
    /// that checks it.
    pub(super) fn check_asserts(&mut self) {
        let ast = self.ast;
        for item in &ast.items {
            let ast::Item::Assert(decl) = item else {
                continue;
            };
            self.annotations(&decl.annotations, annotations::Target::Assert);
            self.outer.push(std::mem::take(&mut self.state));
            self.state.scopes = vec![FxHashMap::default()];
            self.checking_assert = true;
            let root = self.check(decl.assert, Types::UNIT);
            self.checking_assert = false;
            let mut body = std::mem::take(&mut self.state.body);
            body.value = Some(root);
            self.state = self.outer.pop().expect("an assert put the body aside");
            let id = self.program.fns.alloc(FnDef {
                name: Symbol::assert(),
                name_span: decl.span,
                symbol: None,
                receiver: None,
                owner: None,
                interface: None,
                generics: Vec::new(),
                instance_of: None,
                params: Vec::new(),
                ret: Types::UNIT,
                ret_span: None,
                is_extern: false,
                exports_c: false,
                header: None,
                accesses: None,
                is_variadic: false,
                variadic_of: None,
                lends_from: None,
                is_lambda: false,
                generator: None,
                is_tailrec: false,
                is_test: false,
                is_inline: false,
                generated: None,
                intrinsic: None,
                body: Some(body),
                projects: None,
                compile_time: true,
                module: self.current as u32,
                is_pub: false,
                span: decl.span,
            });
            self.program.asserts.push(id);
        }
    }
}
