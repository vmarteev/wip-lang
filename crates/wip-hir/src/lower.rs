//! Name resolution and type checking: syntax tree to [`Program`].
//!
//! Checking is bidirectional. An expected type flows down where the context
//! knows it (annotated `let`, arguments, `return`, field initializers) and is
//! checked with [`Lowerer::check`]; everywhere else the type is synthesized
//! bottom-up by [`Lowerer::infer`]. There is no inference beyond that.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use wip_syntax::ast::{self, Ast};
use wip_syntax::codes;
use wip_syntax::{Diagnostic, Edit, Interner, MAX_TUPLE, MIN_TUPLE, Span, Symbol, parallel};

use crate::PRELUDE;
use crate::hir::*;
use crate::ty::{Ty, TyKind, TyMap, Types};
use crate::{KnownEnum, KnownFn, KnownInterface, KnownStruct};
use aggregates::StructLit;
use arguments::{Matched, Slot};
use generics::{GenericOperand, Inference};
use paths::{ItemUse, PathTarget};
use type_rules::BUILTIN_TYPES;
#[cfg(test)]
use wording::edit_distance;
use wording::{left_the_prelude, plural, suggest};

mod aggregates;
mod annotations;
mod arguments;
mod asserts;
mod body;
mod calls;
mod check;
mod consts;
mod derives;
mod embed;
mod expr;
mod generators;
mod generics;
mod imports;
mod interfaces;
mod items;
mod iterate;
mod lambdas;
mod lists;
mod matching;
mod methods;
mod paths;
mod places;
mod projections;
mod propagate;
mod tailrec;
#[cfg(test)]
mod tests;
mod type_rules;
mod types;
mod usefulness;
mod variadic;
mod wording;

pub struct Lowered {
    pub program: Program,
    pub diagnostics: Vec<Diagnostic>,
}

/// One module's files, as the driver found them.
pub struct ModuleAst<'a> {
    /// `a::b::c`, and empty for the root module. A module of a dependency is
    /// named with the dependency's name first, `engine::render`, which is
    /// how every other package writes it.
    pub path: String,
    pub files: Vec<&'a Ast>,
    /// What the module's own paths are written from: empty for the
    /// program's package and the standard library, and the package's name
    /// for a dependency, so that `render` written in `engine` is
    /// `engine::render`.
    pub prefix: String,
    /// The names of the packages the module's package depends on, which
    /// its paths may begin with as they may begin with `std`.
    pub depends: Vec<String>,
    /// Where its files are, which the files it embeds are read relative
    /// to, and its package's root, which they may not leave.
    /// `None` for the standard library, which is in the
    /// compiler, and for a file checked on its own.
    pub dir: Option<std::path::PathBuf>,
    pub root: Option<std::path::PathBuf>,
}

/// What one file's imports bind.
#[derive(Clone, Default)]
struct FileImports {
    /// Modules, by the name the file gave them.
    modules: FxHashMap<Symbol, usize>,
    /// Items of other modules, by the name the file gave them.
    items: FxHashMap<Symbol, ImportedItem>,
}

#[derive(Clone, Copy)]
enum ImportedItem {
    /// The item's module, and its name there.
    Item(usize, ast::Name),
    /// An import already reported as wrong: its uses stay quiet.
    Broken,
}

/// `type Id = i64`: another name for a type. The body is
/// resolved once, with the alias's own parameters standing in it, and each
/// use substitutes the arguments it was given.
#[derive(Clone)]
struct AliasDef {
    /// `None` until something asks for it, and its body is resolved.
    ty: Option<Ty>,
    generics: Vec<GenericParamDef>,
    is_pub: bool,
    span: Span,
}

/// How far the checker has come through a program, in order: what it
/// learns in one phase it knows in every later one, and a check that
/// needs what it does not know yet waits for the phase that tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    /// Names are declared and types resolved: whether a type owns memory
    /// is not known yet.
    Declaring,
    /// Every type's body is resolved; what implements what is not known
    /// yet.
    TypesResolved,
    /// Every `extend` block is declared, and bodies are checked.
    ImplsDeclared,
}

/// Where an alias is written, to resolve its body there.
#[derive(Clone)]
struct AliasSource<'a> {
    decl: &'a ast::TypeAlias,
    ast: &'a Ast,
    imports: FileImports,
}

/// Where a type whose fields have defaults is declared, with what its file
/// sees: its defaults are checked there, when a literal first needs them
/// or in their turn, whichever comes first.
#[derive(Clone)]
struct DefaultSource<'a> {
    item: items::Declared<'a>,
    module: usize,
    ast: &'a Ast,
    imports: FileImports,
    lambdas: FxHashMap<ast::ExprId, FnId>,
    lambda_envs: FxHashMap<FnId, StructId>,
    generators: FxHashMap<ast::ExprId, StructId>,
}

/// The names one module declares, and what it exports.
#[derive(Clone)]
struct ModuleScope {
    path: String,
    /// The module's package, as [`ModuleAst`] says.
    prefix: String,
    depends: Vec<String>,
    /// Where its files are, and its package's root.
    dir: Option<std::path::PathBuf>,
    root: Option<std::path::PathBuf>,
    types: FxHashMap<Symbol, (TypeDef, Span)>,
    /// The type aliases the module declares.
    aliases: FxHashMap<Symbol, AliasDef>,
    /// The opaque C types the module declares.
    opaques: FxHashMap<Symbol, OpaqueId>,
    /// The interfaces the module declares.
    interfaces: FxHashMap<Symbol, (InterfaceId, Span)>,
    fns: FxHashMap<Symbol, FnId>,
    /// The constants the module declares.
    consts: FxHashMap<Symbol, ConstId>,
    /// The variables C owns that the module declares.
    globals: FxHashMap<Symbol, GlobalId>,
}

/// Lowers a program of one file, in the root module.
pub fn lower_file(ast: &Ast, interner: &Interner) -> Lowered {
    lower(
        &[ModuleAst {
            path: String::new(),
            files: vec![ast],
            prefix: String::new(),
            depends: Vec::new(),
            dir: None,
            root: None,
        }],
        interner,
    )
}

/// Lowers a program, checking function bodies on one thread per core
/// ([`parallel::threads`]).
pub fn lower<'a>(modules: &[ModuleAst<'a>], interner: &'a Interner) -> Lowered {
    lower_with_threads(modules, interner, parallel::threads())
}

/// Lowers a program, checking function bodies on up to `threads` threads.
/// The result is the same for any number of threads.
pub fn lower_with_threads<'a>(
    modules: &[ModuleAst<'a>],
    interner: &'a Interner,
    threads: usize,
) -> Lowered {
    // Every file, with the module it belongs to.
    let mut files: Vec<(usize, &'a Ast)> = Vec::new();
    for (index, module) in modules.iter().enumerate() {
        for &file in &module.files {
            files.push((index, file));
        }
    }
    let program = Program {
        modules: modules.iter().map(|p| p.path.clone()).collect(),
        ..Program::default()
    };
    let Some(&(_, first_file)) = files.first() else {
        return Lowered {
            program,
            diagnostics: Vec::new(),
        };
    };
    let scopes = modules
        .iter()
        .map(|p| ModuleScope {
            path: p.path.clone(),
            prefix: p.prefix.clone(),
            depends: p.depends.clone(),
            dir: p.dir.clone(),
            root: p.root.clone(),
            types: FxHashMap::default(),
            aliases: FxHashMap::default(),
            opaques: FxHashMap::default(),
            interfaces: FxHashMap::default(),
            fns: FxHashMap::default(),
            consts: FxHashMap::default(),
            globals: FxHashMap::default(),
        })
        .collect();
    let mut lowerer = Lowerer::new(first_file, interner, program, Arc::new(scopes));
    // The prelude is declared before anything else, so that a module that
    // declares one of its names is reported.
    let prelude = modules.iter().position(|p| p.path == PRELUDE);
    let declaration_order: Vec<usize> = (0..files.len())
        .filter(|&i| Some(files[i].0) == prelude)
        .chain((0..files.len()).filter(|&i| Some(files[i].0) != prelude))
        .collect();
    // Imports are resolved once, so their errors are reported once.
    let imports: Vec<FileImports> = files
        .iter()
        .map(|&(module, file)| {
            lowerer.enter(module, file, FileImports::default());
            lowerer.resolve_imports()
        })
        .collect();

    // Names first, so that a type may be used before it is declared and by
    // any module that imports it. Interfaces come before types, since a
    // type's parameters may be constrained by one.
    let mut interfaces = vec![Vec::new(); files.len()];
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, FileImports::default());
        interfaces[i] = lowerer.collect_interfaces();
    }
    let mut types = vec![Vec::new(); files.len()];
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, FileImports::default());
        types[i] = lowerer.collect_types();
    }
    // The type aliases' names, before any body names one.
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, FileImports::default());
        lowerer.collect_aliases();
    }
    // With every type named, the items a file imports can be checked before
    // anything uses them.
    let mut checked_imports: Vec<FileImports> = vec![FileImports::default(); files.len()];
    let mut imports = imports;
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, std::mem::take(&mut imports[i]));
        checked_imports[i] = lowerer.check_imported_items(modules);
    }
    let imports = checked_imports;
    // A function for every lambda, before the bodies are checked, and
    // before the constants, whose values may hold one too.
    let mut lambdas: Vec<FxHashMap<ast::ExprId, FnId>> = vec![FxHashMap::default(); files.len()];
    let mut envs: Vec<FxHashMap<FnId, StructId>> = vec![FxHashMap::default(); files.len()];
    // And a struct and its `next` for every loop that is a generator.
    let mut generators: Vec<FxHashMap<ast::ExprId, StructId>> =
        vec![FxHashMap::default(); files.len()];
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, imports[i].clone());
        let (file_lambdas, file_envs) = lowerer.collect_lambdas();
        lambdas[i] = file_lambdas;
        envs[i] = file_envs;
        generators[i] = lowerer.collect_generators();
    }
    let enter = |lowerer: &mut Lowerer<'a>, i: usize| {
        let (module, file) = files[i];
        lowerer.enter(module, file, imports[i].clone());
        lowerer.lambdas = lambdas[i].clone();
        lowerer.lambda_envs = envs[i].clone();
        lowerer.generators = generators[i].clone();
    };
    // Constants are declared before type bodies, since an array's length may
    // be one; each is worked out where it is first needed, with its file's
    // lambdas and generators.
    for &i in &declaration_order {
        enter(&mut lowerer, i);
        lowerer.declare_consts();
    }
    // The environment a named function lent as a closure carries, which
    // holds nothing; made here, so that every thread checking a body sees it.
    lowerer.program.fn_closure_env = Some(lowerer.program.structs.alloc(StructDef {
        is_extern: false,
        is_union: false,
        is_opaque: false,
        is_intrinsic: false,
        header: None,
        accessors: Vec::new(),
        name: lowerer.interner.lambda_symbol(),
        generics: Vec::new(),
        fields: Vec::new(),
        methods: Vec::new(),
        is_tuple: false,
        generator: None,
        is_env: true,
        is_view: false,
        module: 0,
        is_pub: false,
        span: Span::at(0),
    }));
    // An alias's body, once the imports are known: each where it is
    // written, and one another names when that one is resolved, which says
    // where an alias names itself.
    for &i in &declaration_order {
        enter(&mut lowerer, i);
        lowerer.record_alias_sources();
    }
    // The defaults of type parameters, before any alias or type body leaves
    // an argument out. One may leave out another's, so they are read in
    // rounds until a round reads none; what is left rests on itself.
    loop {
        let mut read = false;
        for &i in &declaration_order {
            enter(&mut lowerer, i);
            read |= lowerer.resolve_defaults(&types[i], false);
        }
        if !read {
            break;
        }
    }
    for &i in &declaration_order {
        enter(&mut lowerer, i);
        lowerer.resolve_defaults(&types[i], true);
    }
    // An interface's defaults may name a type whose own were read above,
    // and `Self`.
    for &i in &declaration_order {
        enter(&mut lowerer, i);
        lowerer.resolve_interface_defaults(&interfaces[i]);
    }
    for &i in &declaration_order {
        enter(&mut lowerer, i);
        lowerer.resolve_aliases();
    }
    for (i, types) in types.iter().enumerate() {
        enter(&mut lowerer, i);
        lowerer.resolve_type_bodies(types);
    }
    lowerer.check_recursive_types();
    lowerer.flush_copy_checks();
    // An interface's methods before anything implements it.
    let mut fns: Vec<Vec<BodyWork>> = vec![Vec::new(); files.len()];
    // The parameters with defaults, by file, which are checked once every
    // function is declared.
    let mut param_defaults: Vec<Vec<(FnId, usize, ast::ExprId)>> = vec![Vec::new(); files.len()];
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, imports[i].clone());
        let bodies = &mut fns[i];
        lowerer.declare_interface_methods(&interfaces[i], bodies);
        param_defaults[i].append(&mut lowerer.param_defaults);
    }
    // What every type asks `@derive` for, before any is declared: a field
    // of a type declared in a later file is counted too, and what
    // `@derive` wrote is for the type that asked.
    for &i in &declaration_order {
        lowerer.note_derives(&types[i]);
    }
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, imports[i].clone());
        let more = lowerer.collect_fns();
        fns[i].extend(more);
        param_defaults[i].append(&mut lowerer.param_defaults);
    }
    // Every generator is an `Iterator`, with the interface's other
    // methods.
    for &i in &declaration_order {
        enter(&mut lowerer, i);
        lowerer.declare_generator_impls();
    }
    // An enum whose variants carry nothing gets `count` and `fromIndex`,
    // unless the program declared methods of those names.
    for (i, types) in types.iter().enumerate() {
        enter(&mut lowerer, i);
        lowerer.declare_enum_numbers(types);
    }
    // With every implementation declared, the types that asked for
    // `@derive(Eq)` get the comparison the compiler writes.
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, imports[i].clone());
        lowerer.declare_equality(&types[i]);
    }
    // With every implementation declared, the constraints on the types
    // written before them can be checked.
    lowerer.flush_constraint_checks();
    // Defaults, once every function is declared, since one may be code
    // that calls one; a field's first, since a parameter's may be a
    // literal that leaves one out.
    for (i, types) in types.iter().enumerate() {
        let (module, file) = files[i];
        for &(item, def) in types {
            if item.has_field_defaults() {
                let source = DefaultSource {
                    item,
                    module,
                    ast: file,
                    imports: imports[i].clone(),
                    lambdas: lambdas[i].clone(),
                    lambda_envs: envs[i].clone(),
                    generators: generators[i].clone(),
                };
                lowerer.default_sources.insert(def, source);
            }
        }
    }
    for types in &types {
        for &(_, def) in types {
            lowerer.ensure_defaults(def);
        }
    }
    for (i, defaults) in param_defaults.iter().enumerate() {
        enter(&mut lowerer, i);
        lowerer.check_param_defaults(defaults);
    }
    // Every constant that was not needed by a type is worked out here,
    // and each top-level `assert` checked, to be run once
    // the program is.
    for &i in &declaration_order {
        let (module, file) = files[i];
        lowerer.enter(module, file, imports[i].clone());
        lowerer.check_consts();
        lowerer.check_asserts();
    }

    // Bodies last. Every signature is known by now, so no body waits for
    // another: ranges of them are checked on threads of their own, and
    // merged back in order.
    let withheld = &lowerer.withheld;
    let work: Vec<(usize, BodyWork)> = fns
        .iter()
        .enumerate()
        .flat_map(|(i, fns)| fns.iter().map(move |&body| (i, body)))
        .filter(|(_, body)| !withheld.contains(&body.id))
        .collect();
    let weights: Vec<usize> = work
        .iter()
        .map(|(_, body)| (body.span.hi - body.span.lo) as usize)
        .collect();
    let check = |lowerer: &mut Lowerer<'a>, range: Range<usize>| {
        let mut file = None;
        for &(i, work) in &work[range] {
            if file != Some(i) {
                enter(lowerer, i);
                file = Some(i);
            }
            lowerer.check_fn(work.body, work.id);
        }
    };
    let ranges = parallel::split(&weights, threads);
    if ranges.len() == 1 {
        check(&mut lowerer, 0..work.len());
    } else {
        let mark = lowerer.program.types.mark();
        let workers = parallel::run(ranges.clone(), |range| {
            let mut worker = lowerer.worker();
            check(&mut worker, range.clone());
            (worker, range)
        });
        // The types each worker added, numbered in order, as one lowerer
        // would have numbered them. There are few, so this is quick.
        let workers: Vec<_> = workers
            .into_iter()
            .map(|(worker, range)| {
                let map = lowerer.program.types.absorb(&worker.program.types, mark);
                (worker, range, map)
            })
            .collect();
        // Renumbering the bodies, and freeing each worker's copy of the
        // declarations, is not: that happens on threads again.
        let checked = parallel::run(workers, |(worker, range, map)| {
            worker.into_checked(&work[range], &map)
        });
        for checked in checked {
            for (id, body, projects) in checked.bodies {
                let def = &mut lowerer.program.fns[id];
                def.body = body;
                def.projects = projects;
            }
            for (id, def) in checked.lambdas {
                lowerer.program.fns[id] = def;
            }
            for (id, def) in checked.env_structs {
                lowerer.program.structs[id] = def;
            }
            for (next, def, id, strukt, implementation) in checked.generators {
                lowerer.program.fns[next] = def;
                lowerer.program.structs[id] = strukt;
                if let Some((index, args, conditions)) = implementation {
                    let target = &mut lowerer.program.impls[index];
                    target.args = args;
                    target.conditions = conditions;
                }
            }
            lowerer.program.names.extend(checked.names);
            lowerer.diagnostics.extend(checked.diagnostics);
        }
    }
    // A projection that forwards to another lends what that one lends,
    // which is known only now that every body is checked.
    lowerer.resolve_forwarded();
    // The code of the closures made from named functions, which the threads
    // could only mark.
    lowerer.make_fn_closures();
    // A generator that holds itself would be endlessly large.
    lowerer.check_generator_cycles();
    // An `@inline` function that reaches itself could never be spliced.
    lowerer.check_inline_cycles();
    // With every body checked, each call to a C function that takes more
    // than it declares is pointed at a declaration of what it passes.
    variadic::resolve_variadic_calls(&mut lowerer.program);
    // Later phases share the types read-only, so the field types of every
    // instance are interned now. A program with errors goes no
    // further, and its types may be the endless ones reported.
    if !lowerer.diagnostics.iter().any(Diagnostic::is_error) {
        lowerer.complete_types(0);
    }
    Lowered {
        program: lowerer.program,
        diagnostics: lowerer.diagnostics,
    }
}

/// A body to check: the expression, the function it belongs to, and the
/// span of what was written, which decides how the work is split.
#[derive(Clone, Copy)]
struct BodyWork {
    body: ast::ExprId,
    id: FnId,
    span: Span,
}

/// The bodies one worker checked, with what their projections lend, and
/// what it reported.
struct CheckedBodies {
    bodies: Vec<(FnId, Option<Body>, Option<crate::Lent>)>,
    /// The functions of the lambdas this worker checked: their signatures
    /// are known only once their bodies are.
    lambdas: Vec<(FnId, FnDef)>,
    /// What each closure it checked captured.
    env_structs: Vec<(StructId, StructDef)>,
    /// The generators it checked: each one's `next`, its struct, and what
    /// its implementation of `Iterator` yields.
    generators: Vec<GeneratorChecked>,
    /// The names it read that left no expression.
    names: Vec<(Span, Named)>,
    diagnostics: Vec<Diagnostic>,
}

/// A generator a worker checked: its `next`, its struct, and the index of
/// its implementation of `Iterator` with what that yields and when it
/// holds.
type GeneratorChecked = (
    FnId,
    FnDef,
    StructId,
    StructDef,
    Option<(usize, crate::TyList, Vec<GenericParamDef>)>,
);

/// A method being declared: how it takes its receiver, the span of the word
/// that says so, the type it belongs to, and that type's parameters as this
/// declaration names them.
#[derive(Clone)]
struct Member {
    receiver: Receiver,
    keyword: Span,
    owner: MemberOwner,
    type_params: Vec<GenericParamDef>,
    /// Written in an `extend` of an interface that takes types: `From<i64>`
    /// and `From<Feet>` both declare `from`, and which is meant is decided
    /// by what is passed.
    overloaded: bool,
    /// The writing half of a `lend fn`, whose `&T` result it lends as
    /// `&var T`.
    lent: bool,
}

/// What a method belongs to: a type, or an interface, whose methods are
/// checked with `Self` as their type parameter.
#[derive(Clone, Copy)]
enum MemberOwner {
    Type(TypeDef),
    Interface(InterfaceId),
}

/// What an assignment target ultimately writes to.
enum PlaceRoot {
    Local(LocalId),
    /// Behind a `&` parameter, which only reads.
    ThroughRef,
    /// Through a `ptr<T>`: C's memory, which Wip writes whatever the
    /// pointer was declared.
    ThroughPointer,
    /// Behind a `&var` parameter, which writes the caller's variable.
    ThroughVarRef,
    /// In a constant table, which nothing writes.
    Table(ConstId),
    NotAPlace,
}

/// Why a place must be writable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Writing {
    /// `place = value`.
    Assign,
    /// `&var place`.
    Borrow,
}

struct Lowerer<'a> {
    ast: &'a Ast,
    interner: &'a Interner,
    program: Program,
    diagnostics: Vec<Diagnostic>,
    /// Every module, in the order the driver found them; 0 is the root.
    /// Shared with the workers that check bodies.
    modules: Arc<Vec<ModuleScope>>,
    /// The module whose file is being lowered.
    current: usize,
    /// What the current file's imports bind: names for modules, and for
    /// items of other modules. Imports are file-scoped, as they are in Go
    /// and Odin.
    imports: FileImports,

    /// The body being checked, with everything that belongs to it.
    state: BodyState,
    /// The bodies being checked around it, outermost first: a lambda's body
    /// is checked in the middle of the body that writes it, and a name it
    /// does not declare is looked for in these.
    /// How far the checker has come, which says what it can know yet.
    phase: Phase,
    outer: Vec<BodyState>,
    /// Where each alias is written, by its module and name.
    alias_sources: FxHashMap<(usize, Symbol), AliasSource<'a>>,
    /// The aliases being resolved, innermost last: one asked for again
    /// names itself.
    alias_stack: Vec<(usize, Symbol)>,
    /// Set where a type leaves out an argument whose default is not read
    /// yet, while the defaults are read.
    default_waits: bool,
    /// The types whose field defaults are not checked yet. One is taken out
    /// as its defaults are checked, so a default that needs its own type's
    /// finds none.
    default_sources: FxHashMap<TypeDef, DefaultSource<'a>>,
    /// The constraint checks that are waiting: the type given, what it
    /// must implement, the parameter's name and where it was declared, and
    /// where the type was written.
    pending_constraints: Vec<(Ty, crate::Constraint, Symbol, Span, Span)>,
    /// The function of each lambda of the current file, and the struct its
    /// captures go in.
    lambdas: FxHashMap<ast::ExprId, FnId>,
    lambda_envs: FxHashMap<FnId, StructId>,
    /// The struct of each loop of the current file that is a generator.
    generators: FxHashMap<ast::ExprId, StructId>,
    /// Whether what is being lowered is what `@derive` wrote.
    /// It is placed at the annotation, so it names
    /// nothing for an editor.
    derived: bool,
    /// The parameters with a default, as `declare_fn` finds them: checked
    /// once every function is declared.
    param_defaults: Vec<(FnId, usize, ast::ExprId)>,
    /// The bodies `@derive` wrote for a type with a field that keeps it
    /// from applying, which is reported where the field is written: they
    /// are not checked, and say nothing more.
    withheld: FxHashSet<FnId>,
    /// The type each `@derive` argument was written on, which what it
    /// wrote is for.
    derive_owners: FxHashMap<Span, TypeDef>,
    /// Where each constant was written, which constant it is being worked
    /// out, and which have been worked out already.
    const_sources: FxHashMap<ConstId, consts::ConstSource<'a>>,
    const_stack: Vec<ConstId>,
    consts_done: FxHashSet<ConstId>,
    /// Whether a top-level `assert` is being checked, where a file may be
    /// embedded as in a constant.
    checking_assert: bool,
    /// The files embedded so far, each kept once: by where it is and
    /// whether it is text.
    embedded: FxHashMap<(std::path::PathBuf, bool), ConstId>,
    /// The type parameters of the item being lowered.
    type_params: Vec<GenericParamDef>,
    /// The type whose method is being lowered, which `Self` names.
    self_ty: Option<Ty>,
    /// The module whose `pub` items every file sees without an import.
    prelude: Option<usize>,
    pending_copy: Vec<(Ty, Symbol, Span)>,
}

/// Everything about one body being checked. A lambda's body is checked in
/// the middle of the body around it, so the whole of this is put aside and
/// taken up again as one value: a field that belonged to the body and was
/// left behind would be a body checked against another one's locals.
struct BodyState {
    body: Body,
    /// The names in scope, innermost last.
    scopes: Vec<FxHashMap<Symbol, LocalId>>,
    /// The result type, and where it was written.
    ret: Ty,
    ret_span: Option<Span>,
    /// The keywords of the `defer`s being checked, innermost last.
    defers: Vec<(Span, usize)>,
    /// The loops around the statement being checked, innermost last, and
    /// whether a `break` leaves each. A `defer` records how
    /// many there were, since a jump may not leave it.
    /// The loops around the statement being lowered, innermost last: the
    /// name each was given, if any, and whether it has a `break`.
    loops: Vec<(Option<Symbol>, bool)>,
    /// While a projection is checked: how often its body lends, and the
    /// parameter the yielded place belongs to.
    lends: u32,
    lent_param: Option<u32>,
    /// A `lend` in the projection lends a constant table.
    lent_table: bool,
    /// Checking the writing half of a `lend fn`.
    lent_half: bool,
    /// While a lambda's body is checked: what it has captured so far, and
    /// how a closure holds it — `None` for a plain function value, which may
    /// capture nothing.
    captures: Vec<lambdas::Capture>,
    capture_kind: Option<lambdas::CaptureKind>,
    /// The lists being built, innermost last, which `yield` hands its
    /// value to.
    lists: Vec<lists::ListBuild>,
    /// While a generator's body is checked: what it yields.
    generator: Option<generators::GeneratorBuild>,
    /// Arguments written `name = value` where `name: value` was meant,
    /// reported, and checked as their values.
    named_by_equals: rustc_hash::FxHashSet<ast::ExprId>,
    /// An index handed to a type's `at` as its argument: an integer of any
    /// type there, as anywhere a position is.
    index_operands: rustc_hash::FxHashSet<ast::ExprId>,
    /// Calls of generic defaults waiting for the type arguments of the
    /// literal or the call that uses them.
    unsettled_defaults: Vec<ExprId>,
    /// The lambda about to be checked is a `val`'s whole value, which may
    /// keep what it captures by `&` for as long as the `val` is used.
    keeping_closure: bool,
    /// The bindings of plain data that are aliases while the body is
    /// lowered, and copies after it where nothing writes through them.
    settling: Vec<LocalId>,
    /// How each element of a tuple matched in place binds:
    /// set while an arm's pattern is checked, and taken by the tuple
    /// pattern at its top.
    place_binds: Option<Vec<matching::Binds>>,
    /// A `val`'s or a `var`'s whole value is the `&place` about to be
    /// checked, which gives the variable its type.
    local_reference: bool,
    /// A lambda whose result nothing expected, and no `return` has said
    /// yet: the first `return value` says it.
    inferring_ret: bool,
}

impl Default for BodyState {
    fn default() -> BodyState {
        BodyState {
            body: Body::default(),
            scopes: Vec::new(),
            ret: Types::UNIT,
            ret_span: None,
            defers: Vec::new(),
            loops: Vec::new(),
            lends: 0,
            lent_param: None,
            lent_table: false,
            lent_half: false,
            captures: Vec::new(),
            capture_kind: None,
            lists: Vec::new(),
            generator: None,
            named_by_equals: Default::default(),
            index_operands: Default::default(),
            unsettled_defaults: Vec::new(),
            keeping_closure: false,
            settling: Vec::new(),
            place_binds: None,
            local_reference: false,
            inferring_ret: false,
        }
    }
}

impl<'a> Lowerer<'a> {
    fn new(
        ast: &'a Ast,
        interner: &'a Interner,
        program: Program,
        modules: Arc<Vec<ModuleScope>>,
    ) -> Lowerer<'a> {
        Lowerer {
            ast,
            interner,
            program,
            diagnostics: Vec::new(),
            current: 0,
            imports: FileImports::default(),
            state: BodyState::default(),
            outer: Vec::new(),
            alias_sources: FxHashMap::default(),
            alias_stack: Vec::new(),
            default_waits: false,
            default_sources: FxHashMap::default(),
            pending_constraints: Vec::new(),
            lambdas: FxHashMap::default(),
            lambda_envs: FxHashMap::default(),
            generators: FxHashMap::default(),
            derived: false,
            withheld: FxHashSet::default(),
            param_defaults: Vec::new(),
            derive_owners: FxHashMap::default(),
            const_sources: FxHashMap::default(),
            const_stack: Vec::new(),
            consts_done: FxHashSet::default(),
            checking_assert: false,
            embedded: FxHashMap::default(),
            type_params: Vec::new(),
            self_ty: None,
            prelude: modules.iter().position(|p| p.path == PRELUDE),
            modules,
            phase: Phase::Declaring,
            pending_copy: Vec::new(),
        }
    }

    /// A lowerer that checks bodies on another thread. It starts from this
    /// one's declarations and interns types of its own, which
    /// [`Lowerer::into_checked`] hands back.
    fn worker(&self) -> Lowerer<'a> {
        let mut worker = Lowerer::new(
            self.ast,
            self.interner,
            self.program.clone(),
            Arc::clone(&self.modules),
        );
        // The implementations are all declared by the time a body is
        // checked, so a worker checks a constraint where it is written
        // rather than putting it off.
        worker.phase = self.phase;
        // What it records is its own, and joins the lowerer's afterwards.
        worker.program.names.clear();
        worker
    }

    /// What a worker checked: the bodies of `fns`, with its types renumbered
    /// through `map` ([`Types::absorb`]), and its diagnostics. Its copy of
    /// the declarations is freed here, on the thread that calls this.
    fn into_checked(mut self, fns: &[(usize, BodyWork)], map: &TyMap) -> CheckedBodies {
        let renumber = !map.is_identity();
        let bodies = fns
            .iter()
            .map(|&(_, BodyWork { id, .. })| {
                let def = &mut self.program.fns[id];
                let mut body = def.body.take();
                if renumber && let Some(body) = &mut body {
                    body.map_types(|ty| map.get(ty), |list| map.get_list(list));
                }
                (id, body, def.projects)
            })
            .collect();
        // A closure's environment is a struct the worker filled in.
        let env_structs: Vec<(StructId, StructDef)> = self
            .program
            .structs
            .iter()
            // What it captured, or the type parameters of the generic
            // function it was written in, which a worker gives it too.
            .filter(|(_, def)| def.is_env && (!def.fields.is_empty() || !def.generics.is_empty()))
            .map(|(id, def)| {
                let mut def = def.clone();
                if renumber {
                    for field in &mut def.fields {
                        field.ty = map.get(field.ty);
                    }
                }
                (id, def)
            })
            .collect();
        // A lambda's function was declared before the bodies were checked,
        // and filled in by the worker that checked the body around it.
        let lambdas: Vec<(FnId, FnDef)> = self
            .program
            .fns
            .iter()
            .filter(|(_, def)| def.is_lambda && def.body.is_some())
            .map(|(id, def)| {
                let mut def = def.clone();
                if renumber {
                    for param in &mut def.params {
                        param.ty = map.get(param.ty);
                    }
                    def.ret = map.get(def.ret);
                    if let Some(body) = &mut def.body {
                        body.map_types(|ty| map.get(ty), |list| map.get_list(list));
                    }
                }
                (id, def)
            })
            .collect();
        // A generator's `next`, struct and implementation were declared
        // empty, and filled in by the worker that checked its loop.
        let mut generators = Vec::new();
        for (next, def) in self.program.fns.iter() {
            let Some(id) = def.generator else { continue };
            if def.body.is_none() {
                continue;
            }
            let mut def = def.clone();
            let mut strukt = self.program.structs[id].clone();
            let owner = TypeDef::Struct(id);
            let implementation =
                self.program
                    .impls
                    .iter()
                    .position(|i| i.ty == owner)
                    .map(|index| {
                        let i = &self.program.impls[index];
                        (index, i.args, i.conditions.clone())
                    });
            let implementation = implementation.map(|(index, args, conditions)| {
                let args = if renumber { map.get_list(args) } else { args };
                (index, args, conditions)
            });
            if renumber {
                for param in &mut def.params {
                    param.ty = map.get(param.ty);
                }
                def.ret = map.get(def.ret);
                if let Some(body) = &mut def.body {
                    body.map_types(|ty| map.get(ty), |list| map.get_list(list));
                }
                for field in &mut strukt.fields {
                    field.ty = map.get(field.ty);
                }
                if let Some(generator) = &mut strukt.generator {
                    generator.elem = map.get(generator.elem);
                }
            }
            generators.push((next, def, id, strukt, implementation));
        }
        let names = std::mem::take(&mut self.program.names)
            .into_iter()
            .map(|(span, named)| match named {
                Named::Type(ty) if renumber => (span, Named::Type(map.get(ty))),
                named => (span, named),
            })
            .collect();
        CheckedBodies {
            bodies,
            lambdas,
            env_structs,
            generators,
            names,
            diagnostics: self.diagnostics,
        }
    }
}

impl<'a> Lowerer<'a> {
    // ---- helpers ----

    fn text(&self, sym: Symbol) -> &'a str {
        self.interner.resolve(sym)
    }

    fn kind(&self, ty: Ty) -> TyKind {
        self.program.types.kind(ty)
    }

    fn intern(&mut self, kind: TyKind) -> Ty {
        self.program.types.intern(kind)
    }

    /// The type in backticks, for messages.
    fn ty_name(&self, ty: Ty) -> String {
        format!("`{}`", self.program.ty_name(ty, self.interner))
    }

    fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// The names the module being lowered declares.
    fn types(&self) -> &FxHashMap<Symbol, (TypeDef, Span)> {
        &self.modules[self.current].types
    }

    /// The type aliases of this module.
    fn aliases(&self) -> &FxHashMap<Symbol, AliasDef> {
        &self.modules[self.current].aliases
    }

    fn aliases_mut(&mut self) -> &mut FxHashMap<Symbol, AliasDef> {
        &mut Arc::make_mut(&mut self.modules)[self.current].aliases
    }

    /// Only the declaration passes add names, before any worker shares the
    /// scopes, so this never copies them.
    fn types_mut(&mut self) -> &mut FxHashMap<Symbol, (TypeDef, Span)> {
        &mut Arc::make_mut(&mut self.modules)[self.current].types
    }

    fn interfaces(&self) -> &FxHashMap<Symbol, (InterfaceId, Span)> {
        &self.modules[self.current].interfaces
    }

    fn interfaces_mut(&mut self) -> &mut FxHashMap<Symbol, (InterfaceId, Span)> {
        &mut Arc::make_mut(&mut self.modules)[self.current].interfaces
    }

    fn fns(&self) -> &FxHashMap<Symbol, FnId> {
        &self.modules[self.current].fns
    }

    fn opaques(&self) -> &FxHashMap<Symbol, OpaqueId> {
        &self.modules[self.current].opaques
    }

    fn opaques_mut(&mut self) -> &mut FxHashMap<Symbol, OpaqueId> {
        &mut Arc::make_mut(&mut self.modules)[self.current].opaques
    }

    fn consts(&self) -> &FxHashMap<Symbol, ConstId> {
        &self.modules[self.current].consts
    }

    fn consts_mut(&mut self) -> &mut FxHashMap<Symbol, ConstId> {
        &mut Arc::make_mut(&mut self.modules)[self.current].consts
    }

    fn globals(&self) -> &FxHashMap<Symbol, GlobalId> {
        &self.modules[self.current].globals
    }

    fn globals_mut(&mut self) -> &mut FxHashMap<Symbol, GlobalId> {
        &mut Arc::make_mut(&mut self.modules)[self.current].globals
    }

    fn fns_mut(&mut self) -> &mut FxHashMap<Symbol, FnId> {
        &mut Arc::make_mut(&mut self.modules)[self.current].fns
    }

    /// Moves to a file of `module`, with the imports that file wrote.
    /// Whether the module being lowered is the standard library's, which
    /// alone may name what holds memory that holds nothing, or turn an
    /// `own` into an address.
    fn in_std(&self) -> bool {
        let path = &self.modules[self.current].path;
        path == "std" || path.starts_with("std::")
    }

    fn enter(&mut self, module: usize, ast: &'a Ast, imports: FileImports) {
        self.current = module;
        self.ast = ast;
        self.imports = imports;
    }

    fn duplicate(&mut self, name: ast::Name, first: Span) {
        // Parameters written `_` are as many as are written, and none is
        // named.
        if name.sym == Symbol::ignored() {
            return;
        }
        let diagnostic = Diagnostic::error(
            codes::DUPLICATE_DEFINITION,
            format!("`{}` is defined more than once", self.text(name.sym)),
            name.span,
            "defined again here",
        )
        .with_secondary(first, "first defined here");
        self.report(diagnostic);
    }
}
