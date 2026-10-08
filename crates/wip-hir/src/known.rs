//! The prelude's items the compiler itself uses: the interface `==` goes
//! through, the enum `main` may return, the method that makes a `String`
//! a `str`. Each is declared here once, with what it is for, found where
//! the prelude declares it by the name it has there, and read through
//! [`PreludeItems`] by what it is for.

use rustc_hash::FxHashMap;

use crate::{BinaryOp, EnumId, FnId, InterfaceId, StructId};

/// An enum of the prelude's items of one kind, each with the name the
/// prelude gives it.
macro_rules! known {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident = $text:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)*
        }

        impl $name {
            /// Every one, in the order declared.
            pub const ALL: &'static [$name] = &[$($name::$variant,)*];

            /// What the prelude calls it.
            pub fn name(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }

            /// The one the prelude calls `name`, if any is.
            pub fn named(name: &str) -> Option<$name> {
                Self::ALL.iter().copied().find(|known| known.name() == name)
            }
        }
    };
}

known! {
    /// An interface of the prelude's that the compiler asks for.
    pub enum KnownInterface {
        /// What the compiler calls where a value ends.
        Destroy = "Destroy",
        /// What an error implements to be printed, and what interpolation
        /// writes a value with.
        Text = "Text",
        /// What `?` converts an error through.
        From = "From",
        /// What `==` goes through.
        Eq = "Eq",
        /// What `<` goes through.
        Ord = "Ord",
        /// What `@derive(Clone)` implements.
        Clone = "Clone",
        /// What `+` asks a type of a program's own.
        Add = "Add",
        /// `-` between two values.
        Subtract = "Subtract",
        /// `*`.
        Multiply = "Multiply",
        /// `/`.
        Divide = "Divide",
        /// `%`.
        Remainder = "Remainder",
        /// `-x`.
        Negate = "Negate",
        /// `!x`.
        Not = "Not",
        /// What `x[i]` calls.
        Index = "Index",
        /// What `for` walks a container through.
        Items = "Items",
        /// What `for` walks what lends its elements one at a time through.
        Sequence = "Sequence",
        /// What `for` walks what lends no slice through.
        Iterator = "Iterator",
        /// What `for x in move c` walks: what `c` gives up its elements to.
        IntoIterator = "IntoIterator",
        /// What a map asks a key for.
        Hash = "Hash",
    }
}

known! {
    /// An enum of the prelude's that the compiler makes values of.
    pub enum KnownEnum {
        /// What `fromIndex` answers with, and what an
        /// iterator's `next` does.
        Option = "Option",
        /// What `main` may return.
        Result = "Result",
        /// What `Ord` answers with.
        Ordering = "Ordering",
    }
}

known! {
    /// A struct of the prelude's that the compiler makes values of.
    pub enum KnownStruct {
        /// What a string literal becomes where one is expected.
        String = "String",
    }
}

/// A function of the prelude's that the compiler calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KnownFn {
    /// `String::toStr`, called where a `String` is passed to what takes a
    /// `str`.
    StringToStr,
    /// `String::over`, a `String` over a `str`'s own bytes, made where a
    /// `str` is passed to what takes a `&String`.
    StringOver,
    /// `String::of`, a copy of text, made where a `str` is given where a
    /// `String` is expected.
    StringOf,
    /// What a `main` that returns a `Result` with an `i64` goes through.
    ExitCode,
    /// What a `main` that returns a `Result` with nothing goes through.
    ExitVoid,
}

impl KnownFn {
    /// Every one, in the order declared.
    pub const ALL: &'static [KnownFn] = &[
        KnownFn::StringToStr,
        KnownFn::StringOver,
        KnownFn::StringOf,
        KnownFn::ExitCode,
        KnownFn::ExitVoid,
    ];

    /// What the prelude calls it.
    pub fn name(self) -> &'static str {
        match self {
            KnownFn::StringToStr => "toStr",
            KnownFn::StringOver => "over",
            KnownFn::StringOf => "of",
            KnownFn::ExitCode => "exitCode",
            KnownFn::ExitVoid => "exitVoid",
        }
    }

    /// The struct it is a method of, or nothing for a free function.
    pub fn owner(self) -> Option<KnownStruct> {
        match self {
            KnownFn::StringToStr | KnownFn::StringOver | KnownFn::StringOf => {
                Some(KnownStruct::String)
            }
            KnownFn::ExitCode | KnownFn::ExitVoid => None,
        }
    }

    /// The one the prelude calls `name`, as a method of `owner` or, where
    /// that is nothing, as a free function.
    pub fn named(owner: Option<KnownStruct>, name: &str) -> Option<KnownFn> {
        Self::ALL
            .iter()
            .copied()
            .find(|known| known.owner() == owner && known.name() == name)
    }
}

impl KnownInterface {
    /// The interface behind a binary operator on a type of a program's own.
    /// The operator an interface answers, where it is one of the
    /// arithmetic ones.
    pub fn binary_op(self) -> Option<BinaryOp> {
        match self {
            KnownInterface::Add => Some(BinaryOp::Add),
            KnownInterface::Subtract => Some(BinaryOp::Sub),
            KnownInterface::Multiply => Some(BinaryOp::Mul),
            KnownInterface::Divide => Some(BinaryOp::Div),
            KnownInterface::Remainder => Some(BinaryOp::Rem),
            _ => None,
        }
    }

    pub fn of_binary(op: BinaryOp) -> Option<KnownInterface> {
        match op {
            BinaryOp::Add => Some(KnownInterface::Add),
            BinaryOp::Sub => Some(KnownInterface::Subtract),
            BinaryOp::Mul => Some(KnownInterface::Multiply),
            BinaryOp::Div => Some(KnownInterface::Divide),
            BinaryOp::Rem => Some(KnownInterface::Remainder),
            _ => None,
        }
    }
}

/// The prelude's items the compiler uses, by what each is for: filled in
/// where the prelude declares them, and nothing for one it does not.
#[derive(Debug, Default, Clone)]
pub struct PreludeItems {
    interfaces: FxHashMap<KnownInterface, InterfaceId>,
    enums: FxHashMap<KnownEnum, EnumId>,
    structs: FxHashMap<KnownStruct, StructId>,
    fns: FxHashMap<KnownFn, FnId>,
}

impl PreludeItems {
    pub fn interface(&self, which: KnownInterface) -> Option<InterfaceId> {
        self.interfaces.get(&which).copied()
    }

    pub fn enumeration(&self, which: KnownEnum) -> Option<EnumId> {
        self.enums.get(&which).copied()
    }

    pub fn structure(&self, which: KnownStruct) -> Option<StructId> {
        self.structs.get(&which).copied()
    }

    pub fn function(&self, which: KnownFn) -> Option<FnId> {
        self.fns.get(&which).copied()
    }

    /// Which of these `id` is, if it is one.
    pub fn which_interface(&self, id: InterfaceId) -> Option<KnownInterface> {
        self.interfaces
            .iter()
            .find_map(|(&known, &found)| (found == id).then_some(known))
    }

    pub(crate) fn set_interface(&mut self, which: KnownInterface, id: InterfaceId) {
        self.interfaces.insert(which, id);
    }

    pub(crate) fn set_enumeration(&mut self, which: KnownEnum, id: EnumId) {
        self.enums.insert(which, id);
    }

    pub(crate) fn set_structure(&mut self, which: KnownStruct, id: StructId) {
        self.structs.insert(which, id);
    }

    pub(crate) fn set_function(&mut self, which: KnownFn, id: FnId) {
        self.fns.insert(which, id);
    }
}
