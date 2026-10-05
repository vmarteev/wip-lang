//! `count` and `fromIndex`: what an enum whose variants carry nothing gets.
//!
//! Such an enum is which variant it holds, and nothing else, so the number
//! of its variants is a constant and the variant with a given number is a
//! switch over the numbers it has.
use crate::*;

use super::Builder;

impl Builder<'_> {
    /// `static fn count(): i64`: how many variants the enum has.
    pub(super) fn count_fn(&mut self, count: u64) {
        let ret = self.new_local(Types::I64, LocalKind::Return);
        self.ret = Some(ret);
        self.assign(
            Place::local(ret),
            Rvalue::Use(Operand::Const(Const::Int {
                bits: count as u128,
                ty: Types::I64,
            })),
        );
        self.terminate(Terminator::Return);
    }

    /// `static fn all(): [Self; count]`: each element is the variant with
    /// its number.
    pub(super) fn all_variants_fn(&mut self, ty: Ty, array: Ty) {
        let ret = self.new_local(array, LocalKind::Return);
        self.ret = Some(ret);
        let at = self.new_local(Types::I64, LocalKind::Var);
        for variant in 0..self.variants_of(ty).len() as u32 {
            self.assign(
                Place::local(at),
                Rvalue::Use(Operand::Const(Const::Int {
                    bits: u128::from(variant),
                    ty: Types::I64,
                })),
            );
            let element = Place::local(ret).project(Projection::Index(at));
            self.push(Statement::SetVariant(element, variant));
        }
        self.terminate(Terminator::Return);
    }

    /// `static fn fromIndex(index: i64): Option<Self>`: the variant with
    /// that number, or nothing where the enum has no such variant.
    pub(super) fn index_to_variant_fn(&mut self, ty: Ty, option: Ty) {
        let index = self.new_local(Types::I64, LocalKind::Param);
        self.params.push(index);
        let ret = self.new_local(option, LocalKind::Return);
        self.ret = Some(ret);
        let count = self.variants_of(ty).len() as u32;

        // A number no variant has answers nothing, which is also what the
        // switch's other cases answer.
        let none = self.new_block();
        let found = self.new_local(ty, LocalKind::Var);
        let narrow = self.value(
            Types::I32,
            Rvalue::Cast(Operand::Copy(Place::local(index)), Types::I32),
        );
        let in_range = self.value(
            Types::BOOL,
            Rvalue::Binary(
                BinaryOp::Lt,
                Operand::Copy(Place::local(index)),
                Operand::Const(Const::Int {
                    bits: u128::from(count),
                    ty: Types::I64,
                }),
            ),
        );
        let switch = self.new_block();
        self.terminate(Terminator::Branch {
            cond: in_range,
            then: switch,
            otherwise: none,
        });

        self.switch_to(switch);
        let some = self.new_block();
        let cases: Vec<(u32, BlockId)> = (0..count)
            .map(|variant| {
                let block = self.new_block();
                self.switch_to(block);
                self.push(Statement::SetVariant(Place::local(found), variant));
                self.terminate(Terminator::Goto(some));
                (variant, block)
            })
            .collect();
        self.switch_to(switch);
        self.terminate(Terminator::Switch {
            value: narrow,
            cases,
            // A negative number, which the range test did not rule out.
            otherwise: none,
        });

        // `.Some(found)`, whose payload is the variant that was found.
        self.switch_to(some);
        self.push(Statement::SetVariant(Place::local(ret), SOME));
        let payload = Place::local(ret).project(Projection::VariantField {
            variant: SOME,
            field: 0,
        });
        self.assign(payload, Rvalue::Use(Operand::Copy(Place::local(found))));
        self.terminate(Terminator::Return);

        self.switch_to(none);
        self.push(Statement::SetVariant(Place::local(ret), NONE));
        self.terminate(Terminator::Return);
    }

    /// The variants of an enum type.
    fn variants_of(&self, ty: Ty) -> &[wip_hir::VariantDef] {
        let TyKind::Enum(id, _) = self.kind(ty) else {
            unreachable!("an enum's number is asked of an enum")
        };
        &self.program.enums[id].variants
    }
}

/// Where the prelude's `Option` writes its two variants.
const SOME: u32 = 0;
const NONE: u32 = 1;
