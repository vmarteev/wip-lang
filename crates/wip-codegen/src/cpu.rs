//! The processor a program is built for: by default, every
//! one of its kind that the systems Wip runs on still run on, so that a
//! program built on one machine runs on another; or the compiling machine
//! itself, or a level named.

use cranelift_codegen::settings::Configurable;

/// Which processors the code may run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cpu {
    /// The baseline of the host's kind of processor: [`Level::baseline`].
    #[default]
    Baseline,
    /// The compiling machine, with every feature it has: the program may
    /// not run on another.
    Native,
    /// A level named.
    Level(Level),
}

/// A level of a kind of processor: the features a program may use, as
/// compilers name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Every x86-64 processor: SSE2.
    X86_64,
    /// x86-64 from Intel's Nehalem (2008) and AMD's Bulldozer (2011) on:
    /// SSE4.2 and POPCNT.
    X86_64V2,
    /// x86-64 from 2013 on (Haswell, Excavator): AVX2, BMI, LZCNT and FMA.
    X86_64V3,
    /// x86-64 with AVX-512.
    X86_64V4,
    /// Every 64-bit Arm processor.
    Armv8_0,
    /// Arm from v8.1 on: the atomic instructions of LSE.
    Armv8_1,
    /// Apple's processors, from the M1 on.
    AppleM1,
}

impl Level {
    const X86: [Level; 4] = [
        Level::X86_64,
        Level::X86_64V2,
        Level::X86_64V3,
        Level::X86_64V4,
    ];
    const ARM: [Level; 3] = [Level::Armv8_0, Level::Armv8_1, Level::AppleM1];

    /// What a level is called, as `--cpu` takes it and as C compilers name
    /// it.
    pub fn name(self) -> &'static str {
        match self {
            Level::X86_64 => "x86-64",
            Level::X86_64V2 => "x86-64-v2",
            Level::X86_64V3 => "x86-64-v3",
            Level::X86_64V4 => "x86-64-v4",
            Level::Armv8_0 => "armv8.0",
            Level::Armv8_1 => "armv8.1",
            Level::AppleM1 => "apple-m1",
        }
    }

    /// The levels of the host's kind of processor, oldest first.
    pub fn of_host() -> &'static [Level] {
        if cfg!(target_arch = "x86_64") {
            &Level::X86
        } else if cfg!(target_arch = "aarch64") {
            &Level::ARM
        } else {
            &[]
        }
    }

    /// The level a program is built for when nothing says otherwise: on
    /// x86-64, v2, which every x86-64 processor since Nehalem and Bulldozer
    /// has, and which Windows 11 and Red Hat's systems require; on a Mac with
    /// Apple
    /// silicon, the M1, which every such Mac has; on other Arm systems,
    /// v8.0, which the Raspberry Pi 4 and the first Graviton are.
    pub fn baseline() -> Option<Level> {
        if cfg!(target_arch = "x86_64") {
            Some(Level::X86_64V2)
        } else if cfg!(all(target_arch = "aarch64", target_vendor = "apple")) {
            Some(Level::AppleM1)
        } else if cfg!(target_arch = "aarch64") {
            Some(Level::Armv8_0)
        } else {
            None
        }
    }

    /// Turns on what this level has.
    fn configure(self, isa: &mut dyn Configurable) {
        let features: &[&str] = match self {
            // Cranelift's presets for these are LLVM's.
            Level::X86_64 => &[],
            Level::X86_64V2 => &["x86-64-v2"],
            Level::X86_64V3 => &["x86-64-v3"],
            Level::X86_64V4 => &["x86-64-v4"],
            Level::Armv8_0 => &[],
            Level::Armv8_1 => &["has_lse"],
            Level::AppleM1 => &["has_lse", "has_pauth", "has_fp16", "has_dotprod"],
        };
        for feature in features {
            isa.enable(feature)
                .unwrap_or_else(|err| panic!("`{feature}` is a setting of the host: {err}"));
        }
    }
}

impl Cpu {
    /// What `--cpu` says: `native`, `baseline`, or a level of the host's
    /// kind of processor.
    pub fn parse(text: &str) -> Result<Cpu, String> {
        match text {
            "native" => Ok(Cpu::Native),
            "baseline" => Ok(Cpu::Baseline),
            _ => Level::of_host()
                .iter()
                .find(|level| level.name() == text)
                .map(|&level| Cpu::Level(level))
                .ok_or_else(|| {
                    let names: Vec<&str> = ["baseline", "native"]
                        .into_iter()
                        .chain(Level::of_host().iter().map(|level| level.name()))
                        .collect();
                    format!(
                        "`{text}` is not a processor this compiler builds for; it takes {}",
                        names.join(", ")
                    )
                }),
        }
    }

    /// The level this is, where it is one: the baseline's, or the one
    /// named. The compiling machine's is not a level.
    pub fn level(self) -> Option<Level> {
        match self {
            Cpu::Baseline => Level::baseline(),
            Cpu::Native => None,
            Cpu::Level(level) => Some(level),
        }
    }

    /// The settings of Cranelift's back end for the host: its features as
    /// this says, detected where it is the compiling machine.
    pub(crate) fn isa_builder(self) -> cranelift_codegen::isa::Builder {
        let native = self == Cpu::Native;
        let mut isa = cranelift_native::builder_with_options(native)
            .expect("Cranelift supports the host architecture");
        if let Some(level) = self.level() {
            level.configure(&mut isa);
        }
        isa
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift_codegen::settings;

    /// Whether the back end for `cpu` has the feature called `name`.
    fn has(cpu: Cpu, name: &str) -> bool {
        let isa = cpu
            .isa_builder()
            .finish(settings::Flags::new(settings::builder()))
            .expect("valid settings");
        isa.isa_flags()
            .iter()
            .find(|value| value.name == name)
            .and_then(|value| value.as_bool())
            .unwrap_or_else(|| panic!("`{name}` is a setting of the host"))
    }

    #[test]
    fn the_baseline_is_every_processor_of_the_host_kind() {
        if cfg!(target_arch = "x86_64") {
            assert!(has(Cpu::Baseline, "has_sse42") && has(Cpu::Baseline, "has_popcnt"));
            for later in ["has_avx", "has_avx2", "has_fma", "has_bmi1", "has_lzcnt"] {
                assert!(!has(Cpu::Baseline, later), "`{later}` is after v2");
            }
            assert!(!has(Cpu::Level(Level::X86_64), "has_sse41"));
            assert!(has(Cpu::Level(Level::X86_64V3), "has_fma"));
        }
        if cfg!(all(target_arch = "aarch64", target_vendor = "apple")) {
            assert!(
                has(Cpu::Baseline, "has_lse"),
                "every Mac with Apple silicon has LSE"
            );
        } else if cfg!(target_arch = "aarch64") {
            // A Raspberry Pi 4 has no LSE, and an atomic add is then a
            // loop of two instructions.
            assert!(!has(Cpu::Baseline, "has_lse"));
            assert!(has(Cpu::Level(Level::Armv8_1), "has_lse"));
        }
    }

    #[test]
    fn a_processor_is_named_as_compilers_name_it() {
        assert_eq!(Cpu::parse("native"), Ok(Cpu::Native));
        assert_eq!(Cpu::parse("baseline"), Ok(Cpu::Baseline));
        for &level in Level::of_host() {
            assert_eq!(Cpu::parse(level.name()), Ok(Cpu::Level(level)));
        }
        let refused = Cpu::parse("pentium").expect_err("not a level");
        assert!(refused.contains("baseline, native"), "{refused}");
        assert_eq!(Cpu::Native.level(), None);
        assert_eq!(Cpu::Baseline.level(), Level::baseline());
    }
}
