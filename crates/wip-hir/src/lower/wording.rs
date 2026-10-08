//! Wording for diagnostics: suggestions for misspelt names, and plurals.

/// The candidate closest to `name`, if it is close enough to be a typo. Names
/// shorter than three characters get no suggestions: offering `y` for `x` is
/// noise.
pub(super) fn suggest<'s>(
    name: &str,
    candidates: impl IntoIterator<Item = &'s str>,
) -> Option<&'s str> {
    let limit = name.chars().count() / 3;
    if limit == 0 {
        return None;
    }
    candidates
        .into_iter()
        .filter(|&c| c != name)
        .map(|c| (edit_distance(name, c), c))
        .filter(|&(d, _)| d <= limit)
        .min()
        .map(|(_, c)| c)
}

/// Where a name the prelude gave up went, said as the help
/// for a program that used it without an import: the import that brings it
/// back, or the method that took its place.
pub(super) fn left_the_prelude(name: &str) -> Option<String> {
    let module = match name {
        "Map" | "MapEntry" | "Set" => "std::collections",
        "Mapped" | "Filtered" | "Enumerated" | "Taken" | "Skipped" | "Zipped" | "Walk" => {
            "std::iter"
        }
        "Chars" | "Split" | "Lines" | "ParseError" => "std::text",
        "sort" => {
            return Some("`sort` is a method of a slice and a `Vec`: `values.sort()`".to_string());
        }
        "sortBy" => {
            return Some(
                "`sortBy` is a method of a slice and a `Vec`: `values.sortBy(before)`".to_string(),
            );
        }
        _ => return None,
    };
    Some(format!(
        "`{name}` is in `{module}`: `import {module}::{{{name}}}`"
    ))
}

/// Edit distance counting an adjacent transposition as one edit, since
/// swapped letters (`cuont`, `hieght`) are the most common typo.
pub(super) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

pub(super) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "a" or "an", for a word as it is said: `an \`Iterator\``, `a \`Source\``.
pub(super) fn article(word: &str) -> &'static str {
    match word.trim_start_matches('`').chars().next() {
        Some(c) if "AEIOUaeiou".contains(c) => "an",
        _ => "a",
    }
}
