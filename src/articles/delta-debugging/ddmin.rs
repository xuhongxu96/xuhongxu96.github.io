// Compiles and runs on its own:
//
//     rustc --edition 2024 ddmin.rs && ./ddmin

// ANCHOR: all
use std::collections::HashSet;
use std::iter::successors;

/// An indivisible piece of the input: a char, token, line, etc.
type AtomicUnit = u32;

// ANCHOR: configuration
/// The units we keep. Reduction shrinks this set.
type Configuration = HashSet<AtomicUnit>;
// ANCHOR_END: configuration

// ANCHOR: oracle
#[derive(PartialEq)]
enum Verdict {
    Interesting,    // still triggers the bug
    NotInteresting, // does not trigger the bug or is invalid
}

type Oracle = dyn Fn(&Configuration) -> Verdict;
// ANCHOR_END: oracle

// ANCHOR: loop
/// A candidate removal set
type Delta = HashSet<AtomicUnit>;

/// The main loop of delta debugging
fn reduce<P: Policy>(
    units: Configuration,
    oracle: &Oracle,
    mut policy: P,
) -> Configuration {
    let mut config = units;
    loop {
        let mut reduced = None;

        for delta in policy.propose(&config) {
            // an empty delta would be a no-op
            // that could never make progress.
            assert!(!delta.is_empty());

            let candidate = &config - &delta;
            if oracle(&candidate) == Verdict::Interesting {
                reduced = Some(candidate);
                break;
            }
        }

        match reduced {
            Some(candidate) => config = candidate, // progress; keep going
            None => break,                         // fixpoint reached
        }
    }

    config
}
// ANCHOR_END: loop

// ANCHOR: policy
trait Policy {
    /// Generate candidate removal sets *lazily*.
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta>;
}
// ANCHOR_END: policy

// ANCHOR: partition
/// Split `config` into at most `n` roughly-equal, disjoint subsets.
fn partition(config: &Configuration, n: usize) -> Vec<Delta> {
    let mut items: Vec<AtomicUnit> = config.iter().copied().collect();
    items.sort_unstable(); // deterministic chunks for a reproducible demo
    let len = items.len();
    if n == 0 || len == 0 {
        return Vec::new();
    }
    let size = len.div_ceil(n);
    items
        .chunks(size)
        .map(|c| c.iter().copied().collect())
        .collect()
}
// ANCHOR_END: partition

// ANCHOR: ddmin
struct DDMin; // no state — granularity lives inside one `propose` call

impl Policy for DDMin {
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        let units = config.len();

        // Granularities n = 2, 4, 8, ... up to `units`
        successors(Some(2), move |&n| (n < units).then(|| (2 * n).min(units)))
            .flat_map(move |n| {
                let subsets = partition(config, n); // n roughly-equal subsets
                // First every δ = ∇ᵢ (keep only Δᵢ), then every δ = Δᵢ (drop Δᵢ).
                let keep_only = subsets.clone().into_iter().map(move |d| config - &d);
                keep_only.chain(subsets)
            })
    }
}
// ANCHOR_END: ddmin

// ANCHOR: main
fn main() {
    println!("minimizing the set 1..=8; interesting iff it keeps 2 and 7");

    let input: Configuration = (1..=8).collect();

    let oracle_calls = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let counter = oracle_calls.clone();
    let keeps_2_and_7 = move |c: &Configuration| {
        counter.set(counter.get() + 1);
        let mut probe: Vec<AtomicUnit> = c.iter().copied().collect();
        probe.sort_unstable();
        let verdict = if c.contains(&2) && c.contains(&7) {
            Verdict::Interesting
        } else {
            Verdict::NotInteresting
        };
        let mark = if verdict == Verdict::Interesting {
            "interesting (reduce to this)"
        } else {
            "not interesting"
        };
        println!("  test {probe:?} -> {mark}");
        verdict
    };

    let mut result: Vec<_> = reduce(input, &keeps_2_and_7, DDMin)
        .into_iter()
        .collect();
    result.sort_unstable();
    println!("=> minimized to {result:?} in {} oracle calls", oracle_calls.get());
    assert_eq!(result, [2, 7]);
}
// ANCHOR_END: main
// ANCHOR_END: all
