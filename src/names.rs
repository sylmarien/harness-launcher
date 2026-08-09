//! Naming a spawn.
//!
//! One string names the spawn, its branch and its worktree directory, so the
//! things left on disk identify themselves. Branches outlive spawns — retiring a
//! spawn leaves the branch — so these names get read months later while pruning:
//! `spawn/a7f3` is unreadable by then, `spawn/add-retry-logic-a7f3` is not.
//!
//! The suffix is random rather than a counter because a counter needs state the
//! app does not keep. It also means a path is never reused, so worktree metadata
//! stranded by a crash never blocks anything.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How much of the work description a name keeps.
const SLUG_LIMIT: usize = 32;

/// What a name falls back to when the description has no usable characters.
const FALLBACK_SLUG: &str = "work";

const SUFFIX_LENGTH: usize = 4;
const SUFFIX_ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Name a spawn after the work it was asked to do.
pub fn spawn_name(work: &str, seed: u64) -> String {
    format!("{}-{}", slug(work), suffix(seed))
}

/// The branch a spawn works on, namespaced so it is obvious who made it.
pub fn branch_name(spawn_name: &str) -> String {
    format!("spawn/{spawn_name}")
}

/// A seed for [`spawn_name`], from the only two things to hand that differ
/// between two spawns started at once: the clock, and which process is asking.
///
/// The clock is taken as its two halves rather than as a count of nanoseconds,
/// which would be a `u128` and would have to be cut down to fit. Seconds and
/// sub-second nanoseconds are each already narrow enough to widen into a `u64`,
/// so nothing is thrown away and nothing has to be justified.
pub fn fresh_seed() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let clock = (u64::from(since_epoch.subsec_nanos()) << 32) ^ since_epoch.as_secs();

    clock ^ u64::from(std::process::id())
}

/// The readable half of a name: the work description, as far as it fits.
///
/// Words are kept whole. A description that reaches the limit mid-word stops
/// before it, because a truncated word reads as a typo rather than as a name cut
/// short.
fn slug(work: &str) -> String {
    let mut slug = String::new();

    for word in work.split(|character: char| !character.is_ascii_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let addition = if slug.is_empty() {
            word.len()
        } else {
            word.len() + 1
        };
        if slug.len() + addition > SLUG_LIMIT {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&word.to_ascii_lowercase());
    }

    if slug.is_empty() {
        return FALLBACK_SLUG.to_string();
    }
    slug
}

/// The half that makes a name unique.
///
/// One scrambled byte per character, rather than repeatedly dividing the seed
/// down: a byte widens into an index losslessly, so the arithmetic stays in the
/// type it started in. The alphabet does not divide 256 evenly, so the first few
/// characters are very slightly likelier than the rest — which costs a fraction
/// of a bit across four characters and matters to nothing here, where the suffix
/// only has to make paths distinct.
fn suffix(seed: u64) -> String {
    scramble(seed)
        .to_le_bytes()
        .into_iter()
        .take(SUFFIX_LENGTH)
        .map(|byte| char::from(SUFFIX_ALPHABET[usize::from(byte) % SUFFIX_ALPHABET.len()]))
        .collect()
}

/// Spread a seed's bits about, so two spawns started a microsecond apart do not
/// land on adjacent suffixes. This is [splitmix64], chosen because it is four
/// lines and needs no dependency — it is not, and does not need to be,
/// cryptographic.
///
/// [splitmix64]: https://prng.di.unimi.it/splitmix64.c
fn scramble(seed: u64) -> u64 {
    let mut state = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    state = (state ^ (state >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    state = (state ^ (state >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    state ^ (state >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_reads_as_the_work_it_was_given() {
        assert!(spawn_name("add retry logic", 1).starts_with("add-retry-logic-"));
    }

    #[test]
    fn punctuation_and_case_become_separators() {
        assert!(
            spawn_name("Fix the (worktree) cleanup!", 1).starts_with("fix-the-worktree-cleanup-")
        );
    }

    #[test]
    fn a_long_description_keeps_whole_words_only() {
        let name = spawn_name(
            "resolve the merge conflicts in the authentication middleware",
            1,
        );

        assert!(name.starts_with("resolve-the-merge-conflicts-in-"));
    }

    #[test]
    fn a_description_with_nothing_usable_in_it_still_names_something() {
        assert!(spawn_name("!!! ???", 1).starts_with("work-"));
    }

    #[test]
    fn the_same_seed_names_the_same_spawn() {
        assert_eq!(
            spawn_name("add retry logic", 7),
            spawn_name("add retry logic", 7)
        );
    }

    #[test]
    fn neighbouring_seeds_do_not_produce_neighbouring_names() {
        let names: std::collections::HashSet<String> = (0..1000)
            .map(|seed| spawn_name("same work", seed))
            .collect();

        assert!(
            names.len() > 990,
            "1000 seeds produced only {} distinct names",
            names.len()
        );
    }

    #[test]
    fn a_suffix_is_short_and_safe_in_a_branch_name_and_a_path() {
        let suffix = suffix(42);

        assert_eq!(suffix.len(), SUFFIX_LENGTH);
        assert!(
            suffix
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        );
    }

    #[test]
    fn a_branch_says_who_created_it() {
        assert_eq!(
            branch_name("add-retry-logic-a7f3"),
            "spawn/add-retry-logic-a7f3"
        );
    }
}
