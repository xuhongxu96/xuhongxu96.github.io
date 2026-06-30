// Probabilistic delta debugging: same loop, an adaptive policy. The framework
// below (`reduce`, `Policy`, and the core types) is byte-for-byte identical to
// `ddmin.rs`; only the policy is swapped. Compiles and runs on its own:
//
//     rustc --edition 2024 probdd.rs && ./probdd

// ANCHOR: all
use std::collections::HashMap;
use std::collections::HashSet;

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

// ANCHOR: model
struct ProbDD {
    /// `probs[u]` is the model's belief that `u` is *essential*,
    /// i.e. that it survives into the minimized result.
    probs: HashMap<AtomicUnit, f64>,
    /// The prior probability for a unit we haven't seen before.
    p0: f64,
}

impl ProbDD {
    /// Keep the model in step with the configuration: forget units that are
    /// gone, and give freshly seen units the prior `p0`.
    fn sync(&mut self, config: &Configuration) {
        self.probs.retain(|u, _| config.contains(u));
        for &u in config {
            self.probs.entry(u).or_insert(self.p0);
        }
    }
}
// ANCHOR_END: model

// ANCHOR: choose
/// Choose the removal set with the highest *expected gain*.
fn best_prefix(probs: &HashMap<AtomicUnit, f64>) -> Vec<AtomicUnit> {
    let mut units: Vec<AtomicUnit> = probs.keys().copied().collect();
    // ascending by probability; ties by id for a reproducible demo.
    units.sort_by(|a, b| {
        probs[a].partial_cmp(&probs[b]).unwrap().then(a.cmp(b))
    });

    let mut survive = 1.0; // ∏ (1 - p) over the current prefix
    let (mut best_k, mut best_gain) = (0, 0.0);
    for (i, u) in units.iter().enumerate() {
        survive *= 1.0 - probs[u];
        // The gain is the number of units we expect to remove: k · ∏(1 - p)
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
fn bayes_update(probs: &mut HashMap<AtomicUnit, f64>, pre: &[AtomicUnit]) {
    let survive: f64 = pre.iter().map(|u| 1.0 - probs[u]).product();
    let denom = 1.0 - survive;
    if denom <= 0.0 {
        return;
    }
    for u in pre {
        let p = probs[u];
        probs.insert(*u, (p / denom).min(1.0));
    }
}
// ANCHOR_END: update

// ANCHOR: probdd
impl Policy for ProbDD {
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        self.sync(config);
        let probs = &mut self.probs;

        // The loop only pulls the *next* delta when the previous one failed,
        // so each iteration after the first means "that removal failed".
        let mut last: Option<Vec<AtomicUnit>> = None;
        std::iter::from_fn(move || {
            if let Some(pre) = &last {
                // if the previous removal failed, update the model
                bayes_update(probs, pre);
            }
            // Done once every survivor is believed essential (p = 1).
            if probs.values().all(|&p| p >= 1.0) {
                return None;
            }
            let pre = best_prefix(probs);
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

    let model = ProbDD {
        probs: HashMap::new(),
        p0: 0.1,
    };
    let mut result: Vec<_> = reduce(input, &keeps_2_and_7, model)
        .into_iter()
        .collect();
    result.sort_unstable();
    println!("=> minimized to {result:?} in {} oracle calls", oracle_calls.get());
    assert_eq!(result, [2, 7]);
}
// ANCHOR_END: main
// ANCHOR_END: all
