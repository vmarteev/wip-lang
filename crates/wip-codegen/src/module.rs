//! Module-level code generation: machine types, signatures, declarations,
//! string data, drop functions, and the C entry point.

use super::*;

/// How many words the runtime written in Wip keeps: the
/// allocator's counters, and whether it traces and checks for leaks.
const RUNTIME_WORDS: usize = 8;

/// Whether C is on the other side of a call: a function Wip declares of C,
/// and one Wip exports to C, which crosses the same way in both directions.
pub(super) fn uses_c_abi(def: &FnDef) -> bool {
    def.is_extern || def.exports_c
}

/// The Cranelift type of a register a piece of a struct travels in.
pub(super) fn reg_type(reg: Reg) -> ir::Type {
    match reg {
        Reg::I64 => types::I64,
        Reg::F32 => types::F32,
        Reg::F64 => types::F64,
    }
}

impl<'p> Codegen<'p> {
    pub(super) fn kind(&self, ty: Ty) -> TyKind {
        self.program.types.kind(ty)
    }

    pub(super) fn layout(&mut self, ty: Ty) -> Layout {
        self.layouts.of(self.program, ty)
    }

    pub(super) fn is_aggregate(&self, ty: Ty) -> bool {
        mir::is_aggregate(self.program, ty)
    }

    pub(super) fn scalar_type(&self, ty: Ty) -> Option<ir::Type> {
        match self.kind(ty) {
            _ if mir::is_slice_pointer(self.program, ty) => None,
            TyKind::Int(t) => Some(int_type(t.bits())),
            TyKind::Float(FloatTy::F32) => Some(types::F32),
            TyKind::Float(FloatTy::F64) => Some(types::F64),
            // A C pointer is a word, like every other pointer.
            TyKind::Cstring
            | TyKind::Fn(..)
            | TyKind::Own(_)
            | TyKind::Ptr(_)
            | TyKind::Ref(..) => Some(types::I64),
            TyKind::Bool => Some(types::I8),
            TyKind::Char => Some(types::I32),
            _ => None,
        }
    }

    /// A scalar parameter or result. Values narrower than 32 bits are
    /// extended by the caller, as Apple's arm64 ABI requires and C compilers
    /// assume elsewhere: `bool` and unsigned types with zeros, signed types
    /// with their sign.
    pub(super) fn abi_param(&self, ty: Ty) -> AbiParam {
        let param = AbiParam::new(self.scalar_type(ty).expect("a scalar type"));
        match self.kind(ty) {
            TyKind::Bool => param.uext(),
            TyKind::Int(t) if t.bits() < 32 && t.signed() => param.sext(),
            TyKind::Int(t) if t.bits() < 32 => param.uext(),
            _ => param,
        }
    }

    pub(super) fn signature(&mut self, id: FnId, def: &FnDef) -> Signature {
        if let Some(c_call) = self.c_call(id) {
            return self.c_signature(def, &c_call);
        }
        let params: Vec<Ty> = def.params.iter().map(|p| p.ty).collect();
        self.signature_with(&params, def.ret, uses_c_abi(def))
    }

    /// How C passes `id`'s structs, where it passes one by value and the
    /// compiler makes the call itself; `None` where the call is Wip's
    /// own, crosses only scalars, or goes through a wrapper in C.
    pub(super) fn c_call(&mut self, id: FnId) -> Option<CCall> {
        if let Some(known) = self.c_calls.get(&id) {
            return known.clone();
        }
        let def = &self.program.fns[id];
        let c_call = match self.arch {
            Some(arch)
                if uses_c_abi(def)
                    && !self.shimmed.contains(&id)
                    && mir::c_abi::by_value(self.program, def) =>
            {
                mir::c_abi::classify(self.program, &mut self.layouts, def, arch)
                    .filter(|c_call| !c_call.is_plain())
            }
            _ => None,
        };
        self.c_calls.insert(id, c_call.clone());
        c_call
    }

    /// The signature C gives a function whose structs travel as `c_call`
    /// says: pieces in registers, a pointer to a copy, bytes on the stack,
    /// and a result through registers or the address C returns through.
    fn c_signature(&self, def: &FnDef, c_call: &CCall) -> Signature {
        let mut sig = Signature::new(self.call_conv);
        if c_call.ret == Answer::Hidden {
            sig.params
                .push(AbiParam::special(types::I64, ArgumentPurpose::StructReturn));
        }
        for (param, pass) in def.params.iter().zip(&c_call.params) {
            match pass {
                Pass::Plain if self.is_c_pair(param.ty) => {
                    sig.params.push(AbiParam::new(types::I64));
                    sig.params.push(AbiParam::new(types::I64));
                }
                Pass::Plain => sig.params.push(self.abi_param(param.ty)),
                Pass::Parts(parts) => sig
                    .params
                    .extend(parts.iter().map(|part| AbiParam::new(reg_type(part.reg)))),
                Pass::Spilled { pad, parts } => {
                    sig.params
                        .extend((0..pad.ints).map(|_| AbiParam::new(types::I64)));
                    sig.params
                        .extend((0..pad.floats).map(|_| AbiParam::new(types::F64)));
                    sig.params
                        .extend(parts.iter().map(|part| AbiParam::new(reg_type(part.reg))));
                }
                Pass::Copy => sig.params.push(AbiParam::new(types::I64)),
                Pass::Stack(size) => sig.params.push(AbiParam::special(
                    types::I64,
                    ArgumentPurpose::StructArgument(*size),
                )),
            }
        }
        match &c_call.ret {
            Answer::Plain if self.scalar_type(def.ret).is_some() => {
                sig.returns.push(self.abi_param(def.ret));
            }
            Answer::Parts(parts) => sig
                .returns
                .extend(parts.iter().map(|part| AbiParam::new(reg_type(part.reg)))),
            Answer::Plain | Answer::Hidden => {}
        }
        sig
    }

    /// The signature of a function that takes `params` and returns `ret`,
    /// whether it is called by name or through a value.
    pub(super) fn signature_of(&self, params: &[Ty], ret: Ty) -> Signature {
        self.signature_with(params, ret, false)
    }

    /// Whether the type reaches C as two arguments, a pointer and a
    /// length: a `str` and a `&[T]`.
    ///
    pub(super) fn is_c_pair(&self, ty: Ty) -> bool {
        match self.kind(ty) {
            // A slice is that pair already; `&[T]` is how it is written.
            TyKind::Str | TyKind::Slice(_) => true,
            TyKind::Ref(inner, _) => matches!(self.kind(inner), TyKind::Slice(_)),
            _ => false,
        }
    }

    /// The signature, where `c_abi` says whether C is on the other side: a
    /// `str` is then a pointer and a length rather than one aggregate.
    fn signature_with(&self, params: &[Ty], ret: Ty, c_abi: bool) -> Signature {
        let mut sig = Signature::new(self.call_conv);
        if self.is_aggregate(ret) {
            // The caller passes where to write the result.
            sig.params.push(AbiParam::new(types::I64));
        }
        for &param in params {
            // A `str` and a `&[T]` cross into C as two arguments, a
            // pointer and a length.
            if c_abi && self.is_c_pair(param) {
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(types::I64));
                continue;
            }
            sig.params.push(match self.scalar_type(param) {
                Some(_) => self.abi_param(param),
                // An aggregate, passed by pointer.
                None => AbiParam::new(types::I64),
            });
        }
        if self.scalar_type(ret).is_some() {
            sig.returns.push(self.abi_param(ret));
        }
        sig
    }

    pub(super) fn declare_functions(&mut self) {
        let mut alloc_sig = Signature::new(self.call_conv);
        alloc_sig.params.push(AbiParam::new(types::I64));
        alloc_sig.returns.push(AbiParam::new(types::I64));
        self.alloc_fn = self
            .module
            .declare_function("wip_alloc", Linkage::Import, &alloc_sig)
            .expect("cannot declare `wip_alloc`");

        // Two 128-bit operands as 64-bit halves, the operation, and where to
        // write the result.
        let mut div_sig = Signature::new(self.call_conv);
        for _ in 0..6 {
            div_sig.params.push(AbiParam::new(types::I64));
        }
        self.int128_div_fn = self
            .module
            .declare_function("wip_int128_div", Linkage::Import, &div_sig)
            .expect("cannot declare `wip_int128_div`");

        // A 128-bit integer as halves and flags, to a `double`.
        let mut to_float_sig = Signature::new(self.call_conv);
        for _ in 0..3 {
            to_float_sig.params.push(AbiParam::new(types::I64));
        }
        to_float_sig.returns.push(AbiParam::new(types::F64));
        self.int128_to_float_fn = self
            .module
            .declare_function("wip_int128_to_float", Linkage::Import, &to_float_sig)
            .expect("cannot declare `wip_int128_to_float`");

        // A `double`, signedness, and where to write the 128-bit result.
        let mut from_float_sig = Signature::new(self.call_conv);
        from_float_sig.params.push(AbiParam::new(types::F64));
        from_float_sig.params.push(AbiParam::new(types::I64));
        from_float_sig.params.push(AbiParam::new(types::I64));
        self.float_to_int128_fn = self
            .module
            .declare_function("wip_float_to_int128", Linkage::Import, &from_float_sig)
            .expect("cannot declare `wip_float_to_int128`");

        let mut free_sig = Signature::new(self.call_conv);
        free_sig.params.push(AbiParam::new(types::I64));
        self.free_fn = self
            .module
            .declare_function("wip_free", Linkage::Import, &free_sig)
            .expect("cannot declare `wip_free`");
    }

    /// The object's name for a function of the program or of C, declared
    /// the first time this module refers to it or defines it. A first
    /// reference is noted in `found`: what is referred to is what gets
    /// compiled, and nothing else is.
    pub(super) fn fn_id(&mut self, id: FnId) -> FuncId {
        if let Some(&func) = self.fn_ids.get(id) {
            return func;
        }
        let def = &self.program.fns[id];
        let own = def.module == self.this_module;
        // A generator's `next` is called wherever its iterator is walked,
        // which may be another module: the struct it belongs to is the
        // compiler's, and never `pub`. A default that is
        // code is called wherever a literal leaves its field out, which
        // may be another module too, and so are the
        // prelude's own `indexOfUnsigned` and `indexOfSigned`, wherever an
        // index is of 64 bits or more.
        let shared = def.is_pub
            || def.instance_of.is_some()
            || def.generator.is_some()
            || def.name == wip_syntax::Symbol::default_code()
            || wip_syntax::Symbol::index_of().contains(&def.name)
            || self.program_wide.contains(&id);
        // Another module's private functions cannot be called from
        // here, and are not in any object's exports.
        // What the program as a whole calls is the exception: an
        // instance, a drop and a table's methods belong to no one
        // module.
        // An `@export("C")` function has one plain symbol, which any
        // module may reach, as it may a C function's.
        assert!(
            def.is_extern || def.exports_c || own || shared,
            "`{}` is another module's own",
            self.interner.resolve(def.name)
        );
        // A generic function has no code; its instances do. An `@intrinsic` has
        // none either: the compiler writes its body where it is called. Nor has
        // the `next` of a loop that is a whole function's body: that loop is
        // the function's generator. Nor has what the compiler runs: a
        // constant's initializer, whose answer the program keeps, and a
        // top-level `assert`.
        assert!(
            def.generics.is_empty()
                && def.intrinsic.is_none()
                && !(def.generator.is_some() && def.body.is_none())
                && !def.compile_time,
            "`{}` has no code to refer to",
            self.interner.resolve(def.name)
        );
        // A variadic call made directly goes to the function its
        // declaration names, through its address and with the signature
        // of that call: a symbol has one signature in an object, and
        // `printf` is called with many. A variable C owns is read and
        // written where it is used: it is data, not a function.
        assert!(
            (def.variadic_of.is_none() && def.accesses.is_none()) || self.shimmed.contains(&id),
            "`{}` is not called by its own symbol",
            self.interner.resolve(def.name)
        );
        // `@symbol("sqlite3_open")`: what C calls it, when the
        // declaration named it something else.
        let name = self.interner.resolve(def.symbol.unwrap_or(def.name));
        // Wip functions are prefixed, so they can never collide with a C
        // symbol, including the entry point.
        let (symbol, linkage) = if def.is_extern {
            // A call C has to make for us goes to the shim the
            // compiler wrote, not to the function itself.
            if self.shimmed.contains(&id) {
                (mir::shim_name(id), Linkage::Import)
            } else {
                (name.to_string(), Linkage::Import)
            }
        } else if def.exports_c {
            // `@export("C")`: C knows it by its own name.
            (
                name.to_string(),
                if own {
                    Linkage::Export
                } else {
                    Linkage::Import
                },
            )
        } else {
            // Two modules may each declare `push`, so the symbol carries
            // the module it belongs to.
            // An instance carries its generic function's module: two
            // modules may each declare a generic `push`.
            let home = def
                .instance_of
                .map_or(def.module, |(generic, _)| self.program.fns[generic].module);
            let path = self.program.modules[home as usize].replace("::", ".");
            // A method's symbol carries the type it belongs to: two types
            // of one module may each declare `sum`.
            let owner = self
                .program
                .owner_of_fn(id)
                .map(|owner| self.program.owner_name(owner, self.interner));
            let name = match &owner {
                // A type may declare one name once per implementation
                // of an interface that takes types, so the symbol
                // carries which one.
                Some(owner) if self.overloaded(id) => {
                    format!("{owner}.{name}{}", u32::from(id.into_raw()))
                }
                // Every generator's struct has the one name no
                // program can write, so its `next` carries the number
                // of its function.
                Some(owner) if def.generator.is_some() => {
                    let generic = def.instance_of.map_or(id, |(generic, _)| generic);
                    format!("{owner}.{name}{}", u32::from(generic.into_raw()))
                }
                Some(owner) => format!("{owner}.{name}"),
                // A lambda has no name of its own, so its symbol carries
                // the number of its function.
                None if def.is_lambda => {
                    format!("{name}{}", u32::from(id.into_raw()))
                }
                // Nor has a default that is code: its symbol carries
                // the number of its function, or its generic one's.
                None if def.name == wip_syntax::Symbol::default_code() => {
                    let generic = def.instance_of.map_or(id, |(generic, _)| generic);
                    format!("{name}{}", u32::from(generic.into_raw()))
                }
                None => name.to_string(),
            };
            let mut symbol = if path.is_empty() {
                format!("wip.{name}")
            } else {
                format!("wip.{path}.{name}")
            };
            // An instance is named after its generic function, its type
            // arguments and the module that made it: two modules may
            // each make one, and each compiles its own copy.
            if let Some((_, args)) = def.instance_of {
                let name = self.program.symbol_args(args, self.interner);
                symbol.push_str(&name.replace(' ', ""));
                symbol.push_str(&format!(".p{}", def.module));
            }
            // A module exports what is `pub`, every instance it made
            // and everything the program calls through a drop or a
            // table, keeps the rest to itself, and imports what it calls
            // in other modules.
            let linkage = match (own, shared) {
                (true, true) => Linkage::Export,
                (true, false) => Linkage::Local,
                (false, _) => Linkage::Import,
            };
            (symbol, linkage)
        };
        let sig = self.signature(id, def);
        let func = self
            .module
            .declare_function(&symbol, linkage, &sig)
            .unwrap_or_else(|err| panic!("cannot declare `{symbol}`: {err}"));
        self.fn_ids.insert(id, func);
        self.found.push(Found::Function(id));
        func
    }

    /// One of the runtime's panic functions, declared on first use. Each
    /// takes its own numbers, then the file, the line and the column.
    pub(super) fn panic_fn(&mut self, name: &'static str, numbers: usize) -> FuncId {
        if let Some(&id) = self.panic_fns.get(name) {
            return id;
        }
        let mut sig = Signature::new(self.call_conv);
        // The numbers, then the message or file pointer and its length.
        for _ in 0..numbers + 4 {
            sig.params.push(AbiParam::new(types::I64));
        }
        let id = self
            .module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|err| panic!("cannot declare `{name}`: {err}"));
        self.panic_fns.insert(name, id);
        id
    }

    /// A variable C owns, as the data symbol the linker resolves.
    pub(super) fn c_global(&mut self, global: wip_hir::GlobalId) -> DataId {
        if let Some(&id) = self.c_globals.get(&global) {
            return id;
        }
        let def = &self.program.globals[global];
        let name = self.interner.resolve(def.symbol.unwrap_or(def.name));
        let id = self
            .module
            .declare_data(name, Linkage::Import, def.is_mut, false)
            .unwrap_or_else(|err| panic!("cannot declare `{name}`: {err}"));
        self.c_globals.insert(global, id);
        id
    }

    /// The block of zeroed words the runtime keeps its counters in:
    /// defined in the prelude's object, where the runtime
    /// written in Wip is, and taken from there by any other that inlined
    /// a use of it.
    pub(super) fn runtime_words(&mut self) -> DataId {
        if let Some(id) = self.runtime_words {
            return id;
        }
        const NAME: &str = "wip_runtime_words";
        let defines = self.program.modules[self.this_module as usize] == wip_hir::PRELUDE;
        let linkage = if defines {
            Linkage::Export
        } else {
            Linkage::Import
        };
        let id = self
            .module
            .declare_data(NAME, linkage, true, false)
            .expect("the runtime's words are declared once");
        if defines {
            let mut data = DataDescription::new();
            data.define_zeroinit(RUNTIME_WORDS * 8);
            // An atomic on a word that is not aligned to its size faults
            // on arm64, and a C variable beside it can leave it on four.
            data.set_align(8);
            self.module
                .define_data(id, &data)
                .expect("the runtime's words are defined once");
        }
        self.runtime_words = Some(id);
        id
    }

    /// The runtime's `wip_str_cmp`, which compares two strings' bytes.
    pub(super) fn str_cmp_fn(&mut self) -> FuncId {
        if let Some(&id) = self.panic_fns.get("wip_str_cmp") {
            return id;
        }
        let mut sig = Signature::new(self.call_conv);
        for _ in 0..4 {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("wip_str_cmp", Linkage::Import, &sig)
            .expect("the runtime's functions are declared once");
        self.panic_fns.insert("wip_str_cmp", id);
        id
    }

    /// The runtime's `wip_cstr_len`, which walks a C string to its NUL.
    pub(super) fn cstr_len_fn(&mut self) -> FuncId {
        if let Some(&id) = self.panic_fns.get("wip_cstr_len") {
            return id;
        }
        let mut sig = Signature::new(self.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("wip_cstr_len", Linkage::Import, &sig)
            .expect("the runtime's functions are declared once");
        self.panic_fns.insert("wip_cstr_len", id);
        id
    }

    /// The bytes of a string, without a NUL: a panic's message and file name
    /// are passed as a pointer and a length.
    pub(super) fn text_data(&mut self, text: &str) -> DataId {
        if let Some(&id) = self.texts.get(text) {
            return id;
        }
        let name = format!("wip.text.{}", self.texts.len());
        let id = self
            .module
            .declare_data(&name, Linkage::Local, false, false)
            .expect("text symbols are unique");
        let mut data = DataDescription::new();
        data.define(text.as_bytes().to_vec().into_boxed_slice());
        self.module
            .define_data(id, &data)
            .expect("text data is defined once");
        self.texts.insert(text.to_string(), id);
        id
    }

    /// A file's bytes, embedded: read-only data of their
    /// own, kept once in the object however many tables point at them.
    fn blob_data(&mut self, bytes: &std::sync::Arc<[u8]>) -> DataId {
        let key = std::sync::Arc::as_ptr(bytes) as *const u8 as usize;
        if let Some(&id) = self.blobs.get(&key) {
            return id;
        }
        let name = format!("wip.embedded.{}", self.blobs.len());
        let id = self
            .module
            .declare_data(&name, Linkage::Local, false, false)
            .expect("embedded files' symbols are unique");
        let mut data = DataDescription::new();
        // An object holds a byte at least; an empty file's is never read.
        let contents = match bytes.is_empty() {
            true => vec![0],
            false => bytes.to_vec(),
        };
        data.define(contents.into_boxed_slice());
        self.module
            .define_data(id, &data)
            .expect("an embedded file is defined once");
        self.blobs.insert(key, id);
        id
    }

    /// The read-only, NUL-terminated data object for a string literal.
    pub(super) fn string_data(&mut self, sym: Symbol) -> DataId {
        if let Some(&id) = self.strings.get(&sym) {
            return id;
        }
        let name = format!("wip.str.{}", self.strings.len());
        let id = self
            .module
            .declare_data(&name, Linkage::Local, false, false)
            .expect("string symbols are unique");
        let mut bytes = self.interner.resolve(sym).as_bytes().to_vec();
        bytes.push(0);
        let mut data = DataDescription::new();
        data.define(bytes.into_boxed_slice());
        self.module
            .define_data(id, &data)
            .expect("string data is defined once");
        self.strings.insert(sym, id);
        id
    }

    /// A constant table, kept once, in read-only data: the
    /// object of the module that declares it defines it, and the others
    /// that read it take it from there, which `found` notes, so that a
    /// table nothing reads is in no object.
    pub(super) fn table_data(&mut self, id: wip_hir::ConstId) -> DataId {
        if let Some(&data) = self.tables.get(&id) {
            return data;
        }
        let def = &self.program.consts[id];
        let path = self.program.modules[def.module as usize].replace("::", ".");
        let name = self.interner.resolve(def.name);
        // `#` is in no name a program writes, so no function's symbol is
        // this. The constant's number makes it the one table of its name:
        // a file's text and its bytes are two, both named by the path.
        let number = id.into_raw().into_u32();
        let symbol = if path.is_empty() {
            format!("wip.table#{name}#{number}")
        } else {
            format!("wip.{path}.table#{name}#{number}")
        };
        let defines = def.module == self.this_module;
        let linkage = if defines {
            Linkage::Export
        } else {
            Linkage::Import
        };
        let data = self
            .module
            .declare_data(&symbol, linkage, false, false)
            .unwrap_or_else(|err| panic!("cannot declare `{symbol}`: {err}"));
        if defines {
            let ty = def.ty;
            let value = def
                .value
                .as_ref()
                .expect("only a table that was worked out is kept");
            let little = self.module.isa().endianness() == ir::Endianness::Little;
            let table = mir::table_bytes(
                self.program,
                self.interner,
                &mut self.layouts,
                value,
                ty,
                little,
            );
            let mut description = DataDescription::new();
            description.define(table.bytes.into_boxed_slice());
            description.set_align(u64::from(self.layout(ty).align));
            // A string's bytes are data of their own, whose address the
            // table holds.
            for (offset, sym) in table.strings {
                let string = self.string_data(sym);
                let global = self.module.declare_data_in_data(string, &mut description);
                description.write_data_addr(offset, global, 0);
            }
            // So are a file's bytes.
            for (offset, bytes) in table.blobs {
                let blob = self.blob_data(&bytes);
                let global = self.module.declare_data_in_data(blob, &mut description);
                description.write_data_addr(offset, global, 0);
            }
            self.module
                .define_data(data, &description)
                .expect("a table is defined once");
        }
        self.tables.insert(id, data);
        if !defines {
            self.found.push(Found::Table(id));
        }
        data
    }

    /// A type's table of methods for an interface: the address of each
    /// method, in the interface's order.
    pub(super) fn vtable_data(&mut self, interface: wip_hir::InterfaceId, ty: Ty) -> DataId {
        if let Some(&id) = self.vtables.get(&(interface, ty)) {
            return id;
        }
        let name = format!(
            "wip.vtable.{}.{}",
            self.interner
                .resolve(self.program.interfaces[interface].name),
            self.vtables.len()
        );
        let id = self
            .module
            .declare_data(&name, Linkage::Local, false, false)
            .expect("table symbols are unique");
        let methods = self.vtable_methods(interface, ty);
        let mut data = DataDescription::new();
        // Zeroed bytes, not a zero-init object: the addresses are written
        // into them by relocations.
        data.define(vec![0u8; methods.len() * 8].into_boxed_slice());
        for (slot, method) in methods.into_iter().enumerate() {
            let method = self.fn_id(method);
            let func = self.module.declare_func_in_data(method, &mut data);
            data.write_function_addr((slot * 8) as u32, func);
        }
        self.module
            .define_data(id, &data)
            .expect("a table is defined once");
        self.vtables.insert((interface, ty), id);
        id
    }

    /// The functions of a type's table, worked out when the instances were
    /// made.
    fn vtable_methods(&self, interface: wip_hir::InterfaceId, ty: Ty) -> Vec<FnId> {
        self.program
            .vtables
            .iter()
            .find(|table| table.interface == interface && table.ty == ty)
            .expect("every table a program needs was made with its instances")
            .methods
            .clone()
    }

    /// Whether a type declares this method's name more than once, which an
    /// interface that takes types allows.
    fn overloaded(&self, id: FnId) -> bool {
        let def = &self.program.fns[id];
        let generic = def.instance_of.map_or(id, |(generic, _)| generic);
        let name = self.program.fns[generic].name;
        let Some(owner) = self.program.owner_of_fn(generic) else {
            return false;
        };
        self.program
            .methods_of(owner)
            .iter()
            .filter(|&&other| self.program.fns[other].name == name)
            .count()
            > 1
    }

    /// Lowers a function body to MIR and compiles it.
    /// A function of the program, or a method whose body the compiler
    /// writes, with the body every consumer of the MIR
    /// takes.
    pub(super) fn define_function(&mut self, id: FnId, def: &'p FnDef) {
        let Some(mut body) = mir::function_body(self.program, self.interner, id) else {
            return;
        };
        // A release build's small aggregates as scalars.
        if self.optimizes {
            mir::scalars(self.program, &mut body);
            if cfg!(debug_assertions) {
                mir::validate(&body);
            }
        }
        let signature = self.signature(id, def);
        let c_call = self.c_call(id);
        let func = self.fn_id(id);
        self.define_body(
            func,
            signature,
            &body,
            Written::Program(id),
            uses_c_abi(def),
            c_call,
        );
    }

    fn define_body(
        &mut self,
        func_id: FuncId,
        signature: Signature,
        body: &mir::Body,
        written: Written,
        c_abi: bool,
        c_call: Option<CCall>,
    ) {
        let mut ctx = std::mem::replace(&mut self.ctx, cranelift_codegen::Context::new());
        ctx.func.signature = signature;
        ctx.func.name = UserFuncName::user(0, func_id.as_u32());
        let mut fn_ctx = std::mem::replace(&mut self.fn_ctx, FunctionBuilderContext::new());
        let builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
        let homes = FnCodegen::new(self, builder, body).lower(c_abi, c_call);
        self.fn_ctx = fn_ctx;
        self.module
            .define_function(func_id, &mut ctx)
            .unwrap_or_else(|err| {
                panic!(
                    "Cranelift rejected `{}`: {err:?}",
                    self.written_name(written)
                )
            });
        let at = match written {
            Written::Program(id) => Some(self.program.fns[id].span),
            Written::Drop(_) => None,
        };
        let name = self.written_name(written);
        let variables = homes
            .iter()
            .map(|home| {
                let decl = body.local(home.local);
                debuginfo::DebugVar {
                    source: decl.source.expect("a local kept for the debugger is named"),
                    ty: decl.ty,
                    param: decl.kind == mir::LocalKind::Param,
                    offset: slot_offset(&ctx, home.slot),
                    indirect: home.indirect,
                }
            })
            .collect();
        self.describe(func_id, name, at, &ctx, variables);
        self.module.clear_context(&mut ctx);
        self.ctx = ctx;
    }

    /// The name a body's function goes by in a panic's calls and the
    /// debugger.
    fn written_name(&self, written: Written) -> String {
        match written {
            Written::Program(id) => self.program.fn_name(id, self.interner),
            Written::Drop(ty) => format!("drop<{}>", self.program.ty_name(ty, self.interner)),
        }
    }

    /// Records where each run of a function's code was written, for a
    /// panic's calls and the debugger: the function the
    /// program declared at `at`, or code the compiler wrote on its own.
    fn describe(
        &mut self,
        func: FuncId,
        name: String,
        at: Option<wip_syntax::Span>,
        ctx: &cranelift_codegen::Context,
        variables: Vec<debuginfo::DebugVar>,
    ) {
        let code = ctx
            .compiled_code()
            .expect("a function is described once it is compiled");
        let rows = code
            .buffer
            .get_srclocs_sorted()
            .iter()
            .filter(|run| !run.loc.is_default())
            .map(|run| (run.start, run.loc.bits()))
            .collect();
        self.described.push(debuginfo::DebugFn {
            func,
            name,
            at,
            glue: at.is_none(),
            size: code.code_buffer().len() as u32,
            rows,
            variables,
            unwind: code
                .create_unwind_info(self.module.isa())
                .expect("Cranelift says how to unwind what it compiled"),
        });
    }

    fn drop_signature(&self) -> Signature {
        let mut sig = Signature::new(self.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig
    }

    /// The drop function of an `own` of `ty`, declared on first use and
    /// defined by `define_drop_fns`.
    pub(super) fn drop_fn(&mut self, ty: Ty) -> FuncId {
        if let Some(&id) = self.drop_fns.get(&ty) {
            return id;
        }
        let name = format!("wip.drop.{}", self.drop_fns.len());
        let id = self
            .module
            .declare_function(&name, Linkage::Local, &self.drop_signature())
            .expect("drop function symbols are unique");
        self.drop_fns.insert(ty, id);
        self.pending_drops.push(ty);
        id
    }

    /// The function that drops a value of `ty` where it lies, declared on
    /// first use and defined by `define_drop_fns`.
    pub(super) fn drop_in_place_fn(&mut self, ty: Ty) -> FuncId {
        if let Some(&id) = self.drop_in_place_fns.get(&ty) {
            return id;
        }
        let name = format!("wip.drop_in_place.{}", self.drop_in_place_fns.len());
        let id = self
            .module
            .declare_function(&name, Linkage::Local, &self.drop_signature())
            .expect("drop function symbols are unique");
        self.drop_in_place_fns.insert(ty, id);
        self.pending_drops_in_place.push(ty);
        id
    }

    /// Defines the drop functions used so far, including those they use in
    /// turn. Their bodies are MIR too.
    pub(super) fn define_drop_fns(&mut self) {
        loop {
            if let Some(ty) = self.pending_drops.pop() {
                let func_id = self.drop_fns[&ty];
                let body = mir::lower_drop_fn(self.program, self.interner, ty);
                if cfg!(debug_assertions) {
                    mir::validate(&body);
                }
                let signature = self.drop_signature();
                self.define_body(func_id, signature, &body, Written::Drop(ty), false, None);
            } else if let Some(ty) = self.pending_drops_in_place.pop() {
                let func_id = self.drop_in_place_fns[&ty];
                let body = mir::lower_drop_in_place_fn(self.program, self.interner, ty);
                if cfg!(debug_assertions) {
                    mir::validate(&body);
                }
                let signature = self.drop_signature();
                self.define_body(func_id, signature, &body, Written::Drop(ty), false, None);
            } else {
                break;
            }
        }
    }

    /// The C entry point: calls Wip's `main`, with the program's arguments if
    /// it takes them, and returns its result as the process exit code.
    /// The entry point `wip test` needs: it calls each test in turn,
    /// printing its name before and `ok` after. A test that
    /// fails panics, which prints the message and ends the program, so the
    /// unfinished line names the test that failed.
    pub(super) fn define_test_entry(&mut self, tests: &[FnId]) {
        let test_fns: Vec<FuncId> = tests.iter().map(|&id| self.fn_id(id)).collect();
        let mut sig = Signature::new(self.call_conv);
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let entry = self
            .module
            .declare_function("main", Linkage::Export, &sig)
            .expect("`main` is free because Wip functions are prefixed");
        // Every line the runner prints, made before the function is built
        // so that the texts are defined while nothing borrows the builder.
        let plural = if tests.len() == 1 { "test" } else { "tests" };
        let mut lines = vec![format!("running {} {plural}\n", tests.len())];
        for &id in tests {
            lines.push(format!(
                "test {} ... ",
                self.program.test_name(id, self.interner)
            ));
        }
        lines.push("ok\n".to_string());
        lines.push(format!("\n{} {plural} passed\n", tests.len()));
        let texts: Vec<(DataId, i64)> = lines
            .iter()
            .map(|line| (self.text_data(line), line.len() as i64))
            .collect();
        let print = self.print_fn();
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(0, entry.as_u32());
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut self.fn_ctx);
        let block = b.create_block();
        b.append_block_params_for_function_params(block);
        b.switch_to_block(block);
        let print = self.module.declare_func_in_func(print, b.func);
        // Everything this function refers to, declared in it before it is
        // built: the texts it prints, and the tests it calls.
        let lines: Vec<_> = texts
            .iter()
            .map(|&(data, len)| (self.module.declare_data_in_func(data, b.func), len))
            .collect();
        let callees: Vec<_> = test_fns
            .iter()
            .map(|&func| self.module.declare_func_in_func(func, b.func))
            .collect();
        let say = |b: &mut FunctionBuilder, index: usize| {
            let (global, len) = lines[index];
            let ptr = b.ins().symbol_value(types::I64, global);
            let len = b.ins().iconst(types::I64, len);
            b.ins().call(print, &[ptr, len]);
        };
        say(&mut b, 0);
        for (index, &callee) in callees.iter().enumerate() {
            say(&mut b, index + 1);
            b.ins().call(callee, &[]);
            say(&mut b, tests.len() + 1);
        }
        say(&mut b, tests.len() + 2);
        let code = b.ins().iconst(types::I32, 0);
        b.ins().return_(&[code]);
        b.seal_all_blocks();
        b.finalize(self.module.target_config());
        self.module
            .define_function(entry, &mut ctx)
            .expect("the test entry point is valid");
        self.describe(entry, "main".to_string(), None, &ctx, Vec::new());
    }

    /// The runtime's `wip_print_text`, which the test runner prints with.
    fn print_fn(&mut self) -> FuncId {
        if let Some(&id) = self.panic_fns.get("wip_print_text") {
            return id;
        }
        let mut sig = Signature::new(self.call_conv);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("wip_print_text", Linkage::Import, &sig)
            .expect("cannot declare `wip_print_text`");
        self.panic_fns.insert("wip_print_text", id);
        id
    }

    pub(super) fn define_entry(&mut self, main: FnId) {
        let main_fn = self.fn_id(main);
        let report_fn = self.program.entry_report.map(|report| self.fn_id(report));
        // `int main(int argc, char **argv)`.
        let mut sig = Signature::new(self.call_conv);
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        let entry = self
            .module
            .declare_function("main", Linkage::Export, &sig)
            .expect("`main` is free because Wip functions are prefixed");
        // Where `main`'s result goes, when it hands one back.
        let result_layout = self
            .program
            .entry_report
            .map(|_| self.layout(self.program.fns[main].ret));
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(0, entry.as_u32());
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut self.fn_ctx);
        let block = b.create_block();
        b.append_block_params_for_function_params(block);
        b.switch_to_block(block);
        let callee = self.module.declare_func_in_func(main_fn, b.func);
        let args = if self.program.fns[main].params.is_empty() {
            Vec::new()
        } else {
            // `args: &[cstring]` is a slice: a pointer to C's `argv` and its
            // length, `argc`. Nothing is copied,
            // and `argv[0]` is the program's name.
            let params = b.block_params(block).to_vec();
            let (argc, argv) = (params[0], params[1]);
            let slot =
                b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 3));
            let slice = b.ins().stack_addr(types::I64, slot, 0);
            b.ins().store(MemFlagsData::trusted(), argv, slice, 0);
            let len = b.ins().sextend(types::I64, argc);
            b.ins().store(MemFlagsData::trusted(), len, slice, 8);
            vec![slice]
        };
        // A `main` that returns a `Result` writes it where the caller says,
        // and the prelude turns it into an exit code: the error is reported
        // there, in Wip, by the error itself.
        let code = if let Some(report) = report_fn {
            let layout = result_layout.expect("a result has a layout");
            let slot = b.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                layout.size.max(1),
                layout.align.trailing_zeros() as u8,
            ));
            let result = b.ins().stack_addr(types::I64, slot, 0);
            let mut with_result = vec![result];
            with_result.extend(args);
            b.ins().call(callee, &with_result);
            let report = self.module.declare_func_in_func(report, b.func);
            let call = b.ins().call(report, &[result]);
            let code = b.inst_results(call)[0];
            b.ins().ireduce(types::I32, code)
        } else {
            let call = b.ins().call(callee, &args);
            if self.program.fns[main].ret == Types::I64 {
                let result = b.inst_results(call)[0];
                b.ins().ireduce(types::I32, result)
            } else {
                b.ins().iconst(types::I32, 0)
            }
        };
        b.ins().return_(&[code]);
        b.seal_all_blocks();
        b.finalize(self.module.target_config());
        self.module
            .define_function(entry, &mut ctx)
            .expect("the entry point is valid");
        self.describe(entry, "main".to_string(), None, &ctx, Vec::new());
    }
}

/// How far a stack slot of a compiled function is from its frame pointer,
/// which is where a debugger finds what is kept there.
fn slot_offset(ctx: &cranelift_codegen::Context, slot: StackSlot) -> i64 {
    let layout = ctx
        .compiled_code()
        .and_then(|code| code.buffer.frame_layout())
        .expect("a compiled function has a frame layout");
    i64::from(layout.stackslots[slot].offset) - i64::from(layout.frame_to_fp_offset)
}

/// What a body being compiled is: a function of the program, with the
/// methods the compiler writes for a type, or the drop
/// function of a type.
#[derive(Clone, Copy)]
enum Written {
    Program(FnId),
    Drop(Ty),
}
