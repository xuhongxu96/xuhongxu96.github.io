// HDD chapter, flat baseline: the same program as hdd.rs, but as the raw
// token stream a flat minimizer would really see -- keywords, braces,
// semicolons and all. The oracle now has to check that a candidate still
// *parses* before it can crash. The framework, DDMin, and ProbDD are
// byte-for-byte the same as in ddmin.rs/probdd.rs. Compiles and runs on
// its own:
//
//     rustc --edition 2024 hdd_flat.rs && ./hdd_flat

use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::successors;

/// An indivisible piece of the input: a char, token, line, etc.
/// Different inputs have different atomic units, so the framework fixes
/// no concrete type: anything copyable, hashable, and orderable serves.
trait AtomicUnit: Copy + Eq + std::hash::Hash + Ord {}
impl<T: Copy + Eq + std::hash::Hash + Ord> AtomicUnit for T {}

/// This chapter's atomic unit: a token of the program, identified by
/// its position in source order (0, 1, 2, ...).
type Token = u32;

/// The units we keep. Reduction shrinks this set.
type Configuration<U> = HashSet<U>;

#[derive(PartialEq)]
enum Verdict {
    Interesting,    // still triggers the bug
    NotInteresting, // does not trigger the bug or is invalid
}

type Oracle<U> = dyn Fn(&Configuration<U>) -> Verdict;

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

struct ProbDD<U: AtomicUnit> {
    /// `unit2prob[u]` is the model's belief that `u` is *essential*,
    /// i.e. that it survives into the minimized result.
    unit2prob: HashMap<U, f64>,
    /// The prior probability for a unit we haven't seen before.
    p0: f64,
}

impl<U: AtomicUnit> ProbDD<U> {
    /// Keep the model in step with the configuration: forget units that are
    /// gone, and give freshly seen units the prior `p0`.
    fn sync(&mut self, config: &Configuration<U>) {
        self.unit2prob.retain(|u, _| config.contains(u));
        for &u in config {
            self.unit2prob.entry(u).or_insert(self.p0);
        }
    }
}

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
        // The gain is the number of units we expect to remove: k · ∏(1 - p)
        let gain = (i + 1) as f64 * survive;
        if gain > best_gain {
            (best_k, best_gain) = (i + 1, gain);
        }
    }

    units.truncate(best_k);
    units
}

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

impl<U: AtomicUnit> Policy<U> for ProbDD<U> {
    fn propose(
        &mut self,
        config: &Configuration<U>,
    ) -> impl Iterator<Item = Delta<U>> {
        self.sync(config);
        let unit2prob = &mut self.unit2prob;

        // The loop only pulls the *next* delta when the previous one failed,
        // so each iteration after the first means "that removal failed".
        let mut last: Option<Vec<U>> = None;
        std::iter::from_fn(move || {
            if let Some(pre) = &last {
                // if the previous removal failed, update the model
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

// ANCHOR: tokens
/// The HDD demo's program, but as the raw token stream a flat minimizer
/// really sees: keywords, braces, and semicolons are units too.
const TOKENS: &[&str] = &[
    // fn bar { b1 ; if guard { g ; crash ( ) ; } b2 ; }
    "fn", "bar", "{", "b1", ";", "if", "guard", "{", "g",
    ";", "crash", "(", ")", ";", "}", "b2", ";", "}",
    // fn f2 { s ; s ; } ... fn f6 { s ; s ; }
    "fn", "f2", "{", "s", ";", "s", ";", "}",
    "fn", "f3", "{", "s", ";", "s", ";", "}",
    "fn", "f4", "{", "s", ";", "s", ";", "}",
    "fn", "f5", "{", "s", ";", "s", ";", "}",
    "fn", "f6", "{", "s", ";", "s", ";", "}",
];

/// Render a configuration back into source: the surviving tokens
/// in source order.
fn render(config: &Configuration<Token>) -> String {
    let mut keep: Vec<Token> =
        config.iter().copied().collect();
    keep.sort_unstable();
    keep.iter()
        .map(|&u| TOKENS[u as usize])
        .collect::<Vec<_>>()
        .join(" ")
}
// ANCHOR_END: tokens

// ANCHOR: parser
// A recursive-descent parser for the demo language:
//
//   program := fn_item*
//   fn_item := "fn" IDENT "{" stmt* "}"
//   stmt    := "if" IDENT "{" stmt* "}"
//            | IDENT "(" ")" ";"
//            | IDENT ";"
fn parses(toks: &[&str]) -> bool {
    let mut pos = 0;
    while pos < toks.len() {
        if !fn_item(toks, &mut pos) {
            return false;
        }
    }
    true
}

fn fn_item(toks: &[&str], pos: &mut usize) -> bool {
    eat(toks, pos, "fn")
        && ident(toks, pos)
        && block(toks, pos)
}

fn block(toks: &[&str], pos: &mut usize) -> bool {
    if !eat(toks, pos, "{") {
        return false;
    }
    while toks.get(*pos) != Some(&"}") {
        if *pos >= toks.len() || !stmt(toks, pos) {
            return false;
        }
    }
    eat(toks, pos, "}")
}

fn stmt(toks: &[&str], pos: &mut usize) -> bool {
    if eat(toks, pos, "if") {
        return ident(toks, pos) && block(toks, pos);
    }
    if !ident(toks, pos) {
        return false;
    }
    if eat(toks, pos, "(") && !eat(toks, pos, ")") {
        return false;
    }
    eat(toks, pos, ";")
}

fn eat(
    toks: &[&str],
    pos: &mut usize,
    want: &str,
) -> bool {
    let hit = toks.get(*pos) == Some(&want);
    if hit {
        *pos += 1;
    }
    hit
}

fn ident(toks: &[&str], pos: &mut usize) -> bool {
    let hit = toks.get(*pos).is_some_and(|t| {
        !matches!(
            *t,
            "fn" | "if" | "{" | "}" | "(" | ")" | ";"
        )
    });
    if hit {
        *pos += 1;
    }
    hit
}
// ANCHOR_END: parser

// ANCHOR: main
fn main() {
    // Every token of the program, punctuation included.
    let all: Configuration<Token> =
        (0..TOKENS.len() as Token).collect();

    // The bug is the call `crash ( ) ;` — four consecutive tokens.
    let crash: Token = TOKENS
        .iter()
        .position(|&t| t == "crash")
        .unwrap() as Token;
    let crash_call = crash..crash + 4;

    for (name, run) in
        [("Flat DDMin", 0u8), ("Flat ProbDD", 1u8)]
    {
        println!("\n==================  {name}  ==================");
        let calls =
            std::rc::Rc::new(std::cell::Cell::new(0u32));
        let parse_errors =
            std::rc::Rc::new(std::cell::Cell::new(0u32));
        let (counter, errors) =
            (calls.clone(), parse_errors.clone());
        let crash_call2 = crash_call.clone();
        // Interesting iff the candidate still *parses* and the
        // crash() call survives whole. A flat minimizer knows
        // nothing about syntax, so it must discover both by test.
        let oracle = move |c: &Configuration<Token>| {
            counter.set(counter.get() + 1);
            let keep: Vec<&str> = {
                let mut ks: Vec<Token> =
                    c.iter().copied().collect();
                ks.sort_unstable();
                ks.iter()
                    .map(|&u| TOKENS[u as usize])
                    .collect()
            };
            let (verdict, mark) = if !parses(&keep) {
                errors.set(errors.get() + 1);
                (Verdict::NotInteresting, "doesn't parse (reject)")
            } else if crash_call2
                .clone()
                .all(|u| c.contains(&u))
            {
                (Verdict::Interesting, "crashes (keep)")
            } else {
                (Verdict::NotInteresting, "ok (reject)")
            };
            println!("  test \"{}\"  ->  {mark}", keep.join(" "));
            verdict
        };

        let result = if run == 0 {
            reduce(all.clone(), &oracle, DDMin)
        } else {
            reduce(
                all.clone(),
                &oracle,
                ProbDD {
                    unit2prob: HashMap::new(),
                    p0: 0.1,
                },
            )
        };

        println!(
            "  => minimized to \"{}\"  in {} oracle calls ({} wasted on parse errors)",
            render(&result),
            calls.get(),
            parse_errors.get()
        );
        assert!(crash_call.clone().all(|u| result.contains(&u)));
        assert_eq!(calls.get(), if run == 0 { 253 } else { 80 });
        assert_eq!(
            parse_errors.get(),
            if run == 0 { 249 } else { 68 }
        );
    }

    // Token deltas are strictly more expressive than subtree deltas:
    // stripping the `if guard { ... }` wrapper -- tokens 5, 6, 7 and the
    // matching `}` at 14 -- keeps a valid, crashing program that HDD can
    // never reach. Neither blind policy above found it.
    let optimum: Configuration<Token> =
        [0, 1, 2, 10, 11, 12, 13, 17].into_iter().collect();
    let keep: Vec<&str> = {
        let mut ks: Vec<Token> =
            optimum.iter().copied().collect();
        ks.sort_unstable();
        ks.iter().map(|&u| TOKENS[u as usize]).collect()
    };
    assert!(parses(&keep));
    assert!(crash_call.clone().all(|u| optimum.contains(&u)));
    println!(
        "\nnote: the flat optimum \"{}\" parses and crashes,\n\
         but neither policy found it",
        render(&optimum)
    );
}
// ANCHOR_END: main
