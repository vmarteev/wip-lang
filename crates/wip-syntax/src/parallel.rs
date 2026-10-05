//! Splitting a phase's work across threads.
//!
//! Work is split into contiguous ranges, in order, and the results come back
//! in the same order. A phase that merges them range by range therefore
//! produces what one pass over all the work would, whatever the number of
//! threads: the same symbols, the same types, the same diagnostics.

use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Zero until [`set_threads`] chooses a number.
static THREADS: AtomicUsize = AtomicUsize::new(0);

/// How many threads the parallel phases use: the number [`set_threads`]
/// chose, or one per core.
pub fn threads() -> usize {
    match THREADS.load(Ordering::Relaxed) {
        0 => std::thread::available_parallelism().map_or(1, |n| n.get()),
        n => n,
    }
}

/// Sets how many threads the parallel phases use; 0 means one per core.
pub fn set_threads(n: usize) {
    THREADS.store(n, Ordering::Relaxed);
}

/// Splits items of the given weights into at most `parts` contiguous ranges
/// of about equal weight. There is always at least one range.
pub fn split(weights: &[usize], parts: usize) -> Vec<Range<usize>> {
    let parts = parts.max(1);
    let total: usize = weights.iter().sum();
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut sum = 0;
    for (i, &weight) in weights.iter().enumerate() {
        sum += weight;
        // Close the range once it holds its share of the whole.
        if ranges.len() + 1 < parts && sum * parts >= total * (ranges.len() + 1) {
            ranges.push(start..i + 1);
            start = i + 1;
        }
    }
    if start < weights.len() || ranges.is_empty() {
        ranges.push(start..weights.len());
    }
    ranges
}

/// Runs `work` on each item, each on its own thread, and returns the results
/// in the items' order. The first item runs on the calling thread. A panic
/// in any of them is resumed here, with its own message.
pub fn run<T: Send, R: Send>(items: Vec<T>, work: impl Fn(T) -> R + Sync) -> Vec<R> {
    if items.len() <= 1 {
        return items.into_iter().map(work).collect();
    }
    std::thread::scope(|scope| {
        let work = &work;
        let mut items = items.into_iter();
        let first = items.next().expect("more than one item");
        let workers: Vec<_> = items.map(|item| scope.spawn(move || work(item))).collect();
        let mut results = vec![work(first)];
        results.extend(workers.into_iter().map(|worker| {
            worker
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        }));
        results
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_cover_every_item_in_order() {
        for weights in [
            vec![],
            vec![5],
            vec![1, 1, 1],
            vec![10, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            vec![0, 0, 0, 0],
            (1..100).collect(),
        ] {
            for parts in 1..8 {
                let ranges = split(&weights, parts);
                assert!(!ranges.is_empty() && ranges.len() <= parts.max(1));
                assert_eq!(ranges[0].start, 0);
                assert_eq!(ranges.last().unwrap().end, weights.len());
                for pair in ranges.windows(2) {
                    assert_eq!(pair[0].end, pair[1].start);
                }
            }
        }
    }

    #[test]
    fn ranges_share_the_weight() {
        let weights = vec![1; 1000];
        let ranges = split(&weights, 4);
        let sizes: Vec<usize> = ranges.iter().map(|r| r.len()).collect();
        assert_eq!(sizes, [250, 250, 250, 250]);
    }

    /// Files lexed with interners of their own, absorbed in order, get the
    /// symbols one interner would have given them.
    #[test]
    fn absorbed_interners_number_symbols_as_one() {
        let files = [
            "fn main() = helper(\"hi\")",
            "fn helper(s: str) = s",
            "struct main { helper: i64, hi: str }",
            // The pieces of interpolated text are symbols too.
            "fn shown(s: str) = \"one \\(s) two \\(s) three\"",
        ];
        let mut one = crate::Interner::new();
        let expected: Vec<_> = files
            .iter()
            .map(|file| crate::lex(file, &mut one).tokens)
            .collect();
        let mut merged = crate::Interner::new();
        for (file, expected) in files.iter().zip(&expected) {
            let mut own = crate::Interner::new();
            let mut lexed = crate::lex(file, &mut own);
            lexed.rename_symbols(&merged.absorb(&own));
            assert_eq!(&lexed.tokens, expected);
        }
    }

    #[test]
    fn results_keep_the_order_of_the_items() {
        let squares = run((0..20).collect(), |n: u64| n * n);
        assert_eq!(squares, (0..20).map(|n| n * n).collect::<Vec<_>>());
    }
}
