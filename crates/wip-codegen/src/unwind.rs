//! How to unwind each function's frame, as `.eh_frame`:
//! where its caller's frame and return address are at each point of its
//! code, which Cranelift says of every function it compiles. The linker
//! joins these with C's, and the system's unwinder walks through both —
//! which is how a panic says the calls that led to it, and how `perf` and
//! a debugger unwind a Wip frame without guessing.

use cranelift_codegen::isa::unwind::UnwindInfo;
use cranelift_object::ObjectProduct;
use gimli::RunTimeEndian;
use gimli::write::{
    Address, CommonInformationEntry, EhFrame, EndianVec, FrameTable, RelocationTarget,
};
use object::write::{Relocation, StandardSection};
use object::{RelocationEncoding, RelocationFlags, RelocationKind};

use crate::debuginfo::{DebugFn, DebugSection};

/// Writes an entry for each function of `fns` that Cranelift says how to
/// unwind, under the common entry `cie` its target has.
pub(crate) fn write(
    product: &mut ObjectProduct,
    fns: &[DebugFn],
    cie: Option<CommonInformationEntry>,
) {
    let Some(mut cie) = cie else {
        return;
    };
    // Where each function is, as a distance from the entry: what a program
    // placed anywhere in memory needs, and what compilers write.
    cie.fde_address_encoding = gimli::DW_EH_PE_pcrel | gimli::DW_EH_PE_sdata4;
    let mut table = FrameTable::default();
    let cie = table.add_cie(cie);
    let mut described = false;
    for (index, function) in fns.iter().enumerate() {
        if let Some(UnwindInfo::SystemV(info)) = &function.unwind {
            let start = Address::Symbol {
                symbol: index,
                addend: 0,
            };
            table.add_fde(cie, info.to_fde(start));
            described = true;
        }
    }
    if !described {
        return;
    }
    let mut frames = EhFrame(DebugSection::new(EndianVec::new(RunTimeEndian::Little)));
    table
        .write_eh_frame(&mut frames)
        .expect("unwind information can be written to memory");
    let written = frames.0;
    let section = product.object.section_id(StandardSection::EhFrame);
    product
        .object
        .set_section_data(section, written.data.slice().to_vec(), 8);
    for relocation in &written.relocations {
        let RelocationTarget::Symbol(index) = relocation.target else {
            unreachable!("an entry names only its function")
        };
        let relative = relocation
            .eh_pe
            .is_some_and(|pe| pe.application() == gimli::DW_EH_PE_pcrel);
        let kind = match relative {
            true => RelocationKind::Relative,
            false => RelocationKind::Absolute,
        };
        let (symbol, addend) = entry_target(product, fns[index].func, relocation.addend);
        product
            .object
            .add_relocation(
                section,
                Relocation {
                    offset: relocation.offset as u64,
                    symbol,
                    addend,
                    flags: RelocationFlags::Generic {
                        kind,
                        encoding: RelocationEncoding::Generic,
                        size: relocation.size * 8,
                    },
                },
            )
            .expect("an unwind entry's relocation is one the object takes");
    }
}

/// What an entry's relocation names to reach its function. In ELF, that
/// is the function's section and its place there, as an assembler writes
/// for `.cfi` directives: a function exported from a shared library may be
/// replaced by another library's of the same name, so the linker refuses a
/// distance to its symbol, where a distance within this object is fixed.
/// Mach-O's linker reads `__eh_frame` itself, and takes the symbol.
fn entry_target(
    product: &mut ObjectProduct,
    func: cranelift_module::FuncId,
    addend: i64,
) -> (object::write::SymbolId, i64) {
    let symbol = product.function_symbol(func);
    if product.object.format() != object::BinaryFormat::Elf {
        return (symbol, addend);
    }
    let defined = product.object.symbol(symbol);
    let object::write::SymbolSection::Section(section) = defined.section else {
        unreachable!("a function with unwind information is defined here")
    };
    let offset = defined.value as i64;
    (product.object.section_symbol(section), addend + offset)
}
