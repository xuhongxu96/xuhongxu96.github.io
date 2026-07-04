// Probabilistic delta debugging: same loop, an adaptive policy. The framework
// below (`reduce`, `Policy`, and the core types) is byte-for-byte identical to
// `ddmin.rs`; only the policy is swapped. Compiles and runs on its own:
//
//     rustc --edition 2024 probdd.rs && ./probdd

// ANCHOR: all
use std::collections::HashMap;
use std::collections::HashSet;

// ANCHOR: atomic-unit
/// An indivisible piece of the input: a char, token, line, etc.
trait AtomicUnit: Copy + Eq + std::hash::Hash + Ord {}
impl<T: Copy + Eq + std::hash::Hash + Ord> AtomicUnit for T {}
// ANCHOR_END: atomic-unit

// ANCHOR: configuration
/// The units we keep.
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

// ANCHOR: model
struct ProbDD<U: AtomicUnit> {
    /// `unit2prob[u]`: the belief that `u` is *essential*.
    unit2prob: HashMap<U, f64>,
    /// The prior for unseen units.
    p0: f64,
}

impl<U: AtomicUnit> ProbDD<U> {
    /// Realign the model with `config`.
    fn sync(&mut self, config: &Configuration<U>) {
        self.unit2prob.retain(|u, _| config.contains(u));
        for &u in config {
            self.unit2prob.entry(u).or_insert(self.p0);
        }
    }
}
// ANCHOR_END: model

// ANCHOR: choose
/// Choose the removal set with the highest *expected gain*.
fn best_prefix<U: AtomicUnit>(
    unit2prob: &HashMap<U, f64>,
) -> Vec<U> {
    let mut units: Vec<U> =
        unit2prob.keys().copied().collect();
    // ascending by probability; ties by id for a reproducible demo.
    units.sort_by(|a, b| {
        unit2prob[a]
            .partial_cmp(&unit2prob[b])
            .unwrap()
            .then(a.cmp(b))
    });

    let mut survive = 1.0; // ∏ (1 - p) over the current prefix
    let (mut best_k, mut best_gain) = (0, 0.0);
    for (i, u) in units.iter().enumerate() {
        survive *= 1.0 - unit2prob[u];
        // gain = k · ∏(1 - p)
        let gain = (i + 1) as f64 * survive;
        if gain > best_gain {
            (best_k, best_gain) = (i + 1, gain);
        }
    }

    units.truncate(best_k);
    units
}
// ANCHOR_END: choose

// ANCHOR: update
/// A removal of `pre` just failed.
/// Raise their beliefs by the Bayesian posterior `p / (1 - ∏ (1 - p))`.
fn bayes_update<U: AtomicUnit>(
    unit2prob: &mut HashMap<U, f64>,
    pre: &[U],
) {
    let survive: f64 =
        pre.iter().map(|u| 1.0 - unit2prob[u]).product();
    let denom = 1.0 - survive;
    if denom <= 0.0 {
        return;
    }
    for u in pre {
        let p = unit2prob[u];
        unit2prob.insert(*u, (p / denom).min(1.0));
    }
}
// ANCHOR_END: update

// ANCHOR: probdd
impl<U: AtomicUnit> Policy<U> for ProbDD<U> {
    fn propose(
        &mut self,
        config: &Configuration<U>,
    ) -> impl Iterator<Item = Delta<U>> {
        self.sync(config);
        let unit2prob = &mut self.unit2prob;

        // pulling the next delta means the previous one failed
        let mut last: Option<Vec<U>> = None;
        std::iter::from_fn(move || {
            if let Some(pre) = &last {
                bayes_update(unit2prob, pre);
            }
            // Done once every survivor is believed essential (p = 1).
            if unit2prob.values().all(|&p| p >= 1.0) {
                return None;
            }
            let pre = best_prefix(unit2prob);
            if pre.is_empty() {
                return None;
            }
            last = Some(pre.clone());
            Some(pre.into_iter().collect())
        })
    }
}
// ANCHOR_END: probdd

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

    let model = ProbDD {
        unit2prob: HashMap::new(),
        p0: 0.1,
    };
    let mut result: Vec<_> =
        reduce(input, &keeps_2_and_7, model)
            .into_iter()
            .collect();
    result.sort_unstable();
    println!(
        "=> minimized to {result:?} in {} oracle calls",
        oracle_calls.get()
    );
    assert_eq!(result, [2, 7]);
    assert_eq!(oracle_calls.get(), 12);
}
// ANCHOR_END: main
// ANCHOR_END: all
