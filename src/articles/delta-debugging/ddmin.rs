// Compiles and runs on its own:
//
//     rustc --edition 2024 ddmin.rs && ./ddmin

// ANCHOR: all
use std::collections::HashSet;
use std::iter::successors;

// ANCHOR: atomic-unit
/// An indivisible piece of the input: a char, token, line, etc.
/// Different inputs have different atomic units, so the framework fixes
/// no concrete type: anything copyable, hashable, and orderable serves.
trait AtomicUnit: Copy + Eq + std::hash::Hash + Ord {}
impl<T: Copy + Eq + std::hash::Hash + Ord> AtomicUnit
    for T
{
}
// ANCHOR_END: atomic-unit

// ANCHOR: configuration
/// The units we keep. Reduction shrinks this set.
type Configuration<U> = HashSet<U>;
// ANCHOR_END: configuration

// ANCHOR: oracle
#[derive(PartialEq)]
enum Verdict {
    Interesting,    // still triggers the bug
    NotInteresting, // does not trigger the bug or is invalid
}

type Oracle<U> = dyn Fn(&Configuration<U>) -> Verdict;
// ANCHOR_END: oracle

// ANCHOR: loop
/// A candidate removal set
type Delta<U> = HashSet<U>;

/// The main loop of delta debugging
fn reduce<U: AtomicUnit, P: Policy<U>>(
    units: Configuration<U>,
    oracle: &Oracle<U>,
    mut policy: P,
) -> Configuration<U> {
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

        // the policy decides when to stop
        let keep_going =
            policy.on_reduced(reduced.as_ref());

        if let Some(candidate) = reduced {
            config = candidate; // update the current configuration
        }

        if !keep_going {
            break;
        }
    }

    config
}
// ANCHOR_END: loop

// ANCHOR: policy
trait Policy<U: AtomicUnit> {
    /// Generate candidate removal sets *lazily*.
    fn propose(
        &mut self,
        config: &Configuration<U>,
    ) -> impl Iterator<Item = Delta<U>>;

    /// React to a reduction pass.
    /// `reduced` is `Some` if the pass removed anything,
    /// `None` if it made no progress.
    /// Return `true` to keep going, `false` to stop.
    /// The default stops at the fixpoint.
    fn on_reduced(
        &mut self,
        reduced: Option<&Configuration<U>>,
    ) -> bool {
        reduced.is_some()
    }
}
// ANCHOR_END: policy

// ANCHOR: partition
/// Split `config` into at most `n` roughly-equal, disjoint subsets.
fn partition<U: AtomicUnit>(
    config: &Configuration<U>,
    n: usize,
) -> Vec<Delta<U>> {
    let mut items: Vec<U> =
        config.iter().copied().collect();
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

impl<U: AtomicUnit> Policy<U> for DDMin {
    fn propose(
        &mut self,
        config: &Configuration<U>,
    ) -> impl Iterator<Item = Delta<U>> {
        let units = config.len();

        // Granularities n = 2, 4, 8, ... up to `units`
        successors(Some(2), move |&n| {
            (n < units).then(|| (2 * n).min(units))
        })
        .flat_map(move |n| {
            let subsets = partition(config, n); // n roughly-equal subsets
            let keep_only = subsets
                .clone()
                .into_iter()
                .map(move |d| config - &d);

            // First every δ = ∇ᵢ (keep only Δᵢ),
            // then every δ = Δᵢ (drop Δᵢ).
            keep_only.chain(subsets)
        })
        .filter(|delta| !delta.is_empty())
    }
}
// ANCHOR_END: ddmin

// ANCHOR: main
fn main() {
    println!("minimizing the set 1..=8; interesting iff it keeps 2 and 7");

    let input: Configuration<u32> = (1..=8).collect();

    let oracle_calls =
        std::rc::Rc::new(std::cell::Cell::new(0u32));
    let counter = oracle_calls.clone();
    let keeps_2_and_7 = move |c: &Configuration<u32>| {
        counter.set(counter.get() + 1);
        let mut probe: Vec<u32> =
            c.iter().copied().collect();
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

    let mut result: Vec<_> =
        reduce(input, &keeps_2_and_7, DDMin)
            .into_iter()
            .collect();
    result.sort_unstable();
    println!(
        "=> minimized to {result:?} in {} oracle calls",
        oracle_calls.get()
    );
    assert_eq!(result, [2, 7]);
    assert_eq!(oracle_calls.get(), 33);
}
// ANCHOR_END: main
// ANCHOR_END: all
