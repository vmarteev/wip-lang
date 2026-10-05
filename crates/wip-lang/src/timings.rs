//! How long each phase of a build takes, for `wip build --time` and the
//! benchmarks.

use super::*;

/// How long each phase of a build took, in the order the phases ran.
#[derive(Default)]
pub struct Timings {
    pub phases: Vec<(&'static str, Duration)>,
}

impl Timings {
    /// Runs `phase`, recording how long it took. A phase that runs again,
    /// such as lexing for each file, adds to its first entry.
    pub fn time<T>(&mut self, phase: &'static str, run: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let value = run();
        let elapsed = start.elapsed();
        match self.phases.iter_mut().find(|(name, _)| *name == phase) {
            Some((_, time)) => *time += elapsed,
            None => self.phases.push((phase, elapsed)),
        }
        value
    }

    /// The time spent in `phase`, or zero if it did not run.
    pub fn get(&self, phase: &str) -> Duration {
        self.phases
            .iter()
            .filter(|(name, _)| *name == phase)
            .map(|(_, time)| *time)
            .sum()
    }

    pub fn total(&self) -> Duration {
        self.phases.iter().map(|(_, time)| *time).sum()
    }
}

impl std::fmt::Display for Timings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |time: Duration| time.as_secs_f64() * 1000.0;
        for (phase, time) in &self.phases {
            writeln!(f, "{phase:<12}{:>10.1} ms", ms(*time))?;
        }
        writeln!(f, "{:<12}{:>10.1} ms", "total", ms(self.total()))
    }
}
