// Hierarchical delta debugging: HDD is *itself* a Policy, driven by the same
// single `reduce` loop as DDMin/ProbDD. Internally its `propose` walks the parse
// tree level by level and delegates candidate generation to an inner Policy
// (DDMin, ProbDD, ...). Compiles and runs on its own:
//
//     rustc --edition 2024 hdd.rs && ./hdd

// ANCHOR: all
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::successors;

/// An indivisible piece of the input: here, a node of the syntax tree.
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
                let keep_only =
                    subsets.clone().into_iter().map(move |d| config - &d);
                keep_only.chain(subsets)
            })
    }
}
// ANCHOR_END: ddmin

// ANCHOR: probdd-model
struct ProbDD {
    probs: HashMap<AtomicUnit, f64>,
    p0: f64,
}

impl ProbDD {
    fn sync(&mut self, config: &Configuration) {
        self.probs.retain(|u, _| config.contains(u));
        for &u in config {
            self.probs.entry(u).or_insert(self.p0);
        }
    }
}
// ANCHOR_END: probdd-model

// ANCHOR: probdd-choose
fn best_prefix(probs: &HashMap<AtomicUnit, f64>) -> Vec<AtomicUnit> {
    let mut units: Vec<AtomicUnit> = probs.keys().copied().collect();
    units.sort_by(|a, b| probs[a].partial_cmp(&probs[b]).unwrap().then(a.cmp(b)));

    let mut survive = 1.0;
    let (mut best_k, mut best_gain) = (0, 0.0);
    for (i, u) in units.iter().enumerate() {
        survive *= 1.0 - probs[u];
        let gain = (i + 1) as f64 * survive;
        if gain > best_gain {
            (best_k, best_gain) = (i + 1, gain);
        }
    }

    units.truncate(best_k);
    units
}
// ANCHOR_END: probdd-choose

// ANCHOR: probdd-update
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
// ANCHOR_END: probdd-update

// ANCHOR: probdd
impl Policy for ProbDD {
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        self.sync(config);
        let probs = &mut self.probs;

        let mut last: Option<Vec<AtomicUnit>> = None;
        std::iter::from_fn(move || {
            if let Some(pre) = &last {
                bayes_update(probs, pre);
            }
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

// ANCHOR: tree
struct Node {
    label: &'static str,
    children: Vec<AtomicUnit>,
}

struct Tree {
    nodes: HashMap<AtomicUnit, Node>,
    root: AtomicUnit,
    depth: HashMap<AtomicUnit, usize>,  // BFS depth of each node
    parent: HashMap<AtomicUnit, AtomicUnit>, // each node's parent
    max_depth: usize,
}
// ANCHOR_END: tree

impl Tree {
    fn new(root: AtomicUnit, nodes: HashMap<AtomicUnit, Node>) -> Tree {
        // BFS from the root to label every node with its depth (= its level)
        // and remember its parent.
        let mut depth = HashMap::new();
        let mut parent = HashMap::new();
        let mut max_depth = 0;
        let mut frontier = vec![root];
        let mut d = 0;
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for &id in &frontier {
                depth.insert(id, d);
                max_depth = d;
                for &c in &nodes[&id].children {
                    parent.insert(c, id);
                    next.push(c);
                }
            }
            frontier = next;
            d += 1;
        }
        Tree { nodes, root, depth, parent, max_depth }
    }

    // ANCHOR: tree-ops
    /// The present leaves in the subtree rooted at `id` (for a leaf, itself).
    /// This is how a removed node turns into a removal of atomic units.
    fn leaves_under(&self, id: AtomicUnit, present: &Configuration) -> Delta {
        let mut out = Delta::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[&n];
            if node.children.is_empty() {
                if present.contains(&n) {
                    out.insert(n);
                }
            } else {
                stack.extend(node.children.iter().copied());
            }
        }
        out
    }

    /// The level-`level` subtrees that still hold a surviving leaf: each present
    /// leaf is represented by its ancestor at that level.
    fn alive_level_nodes(&self, level: usize, present: &Configuration) -> Configuration {
        present
            .iter()
            .copied()
            .filter(|leaf| self.depth[leaf] >= level)
            .map(|leaf| self.ancestor_at(leaf, level))
            .collect()
    }

    /// Walk up from `id` to its ancestor sitting at `level`.
    fn ancestor_at(&self, mut id: AtomicUnit, level: usize) -> AtomicUnit {
        while self.depth[&id] > level {
            id = self.parent[&id];
        }
        id
    }
    // ANCHOR_END: tree-ops
}

// ANCHOR: hdd
/// HDD is a Policy. It walks the tree level by level (coarse → fine) and, for
/// the current level, lets a *fresh inner policy* choose which of that level's
/// subtrees to drop. The configuration is leaves only, so each proposed subtree
/// removal is mapped down to the leaves under it. The single `reduce` loop tests
/// the resulting leaf removals.
struct Hdd<'t, F> {
    tree: &'t Tree,
    new_minimizer: F, // build a fresh list-minimizer for a level, e.g. `|| DDMin`
    level: usize,     // the shallowest level not yet known to be minimal
}

impl<'t, F, P> Policy for Hdd<'t, F>
where
    F: Fn() -> P,
    P: Policy,
{
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        let tree = self.tree;
        let new_minimizer = &self.new_minimizer;
        let level = &mut self.level; // resume progress at the last level that was not minimal
        let start = *level;
        let max = tree.max_depth;

        // One lazy stream chained across levels: only when the outer loop has
        // pulled (and failed) every candidate of level L do we advance to L+1.
        (start..=max).flat_map(move |l| {
            *level = l; // remember how far the single loop has walked
            // Hand the inner minimizer the level's *subtrees* (not raw leaves),
            // so it removes whole subtrees and HDD stays hierarchical.
            let here = tree.alive_level_nodes(l, config);
            let mut inner = new_minimizer(); // a fresh minimizer, just for this level
            inner
                .propose(&here)
                .map(|drop| -> Delta {
                    // dropping a subtree drops the leaves under it
                    drop.iter().flat_map(|&id| tree.leaves_under(id, config)).collect()
                })
                .filter(|delta| !delta.is_empty()) // skip no-op "keep everything" probes
                .collect::<Vec<Delta>>()
                .into_iter()
        })
    }
}
// ANCHOR_END: hdd

/// Render a configuration (a set of leaves) back into nested source-ish text:
/// a node is shown iff some leaf under it survives.
fn render(tree: &Tree, present: &Configuration) -> String {
    fn alive(tree: &Tree, id: AtomicUnit, present: &Configuration) -> bool {
        let node = &tree.nodes[&id];
        if node.children.is_empty() {
            present.contains(&id)
        } else {
            node.children.iter().any(|&c| alive(tree, c, present))
        }
    }
    fn go(tree: &Tree, id: AtomicUnit, present: &Configuration, out: &mut String) {
        let node = &tree.nodes[&id];
        out.push_str(node.label);
        let kids: Vec<AtomicUnit> = node
            .children
            .iter()
            .copied()
            .filter(|&c| alive(tree, c, present))
            .collect();
        if !kids.is_empty() {
            out.push_str(" { ");
            for (i, c) in kids.iter().enumerate() {
                if i > 0 {
                    out.push_str("; ");
                }
                go(tree, *c, present, out);
            }
            out.push_str(" }");
        }
    }
    if !alive(tree, tree.root, present) {
        return String::new();
    }
    let mut out = String::new();
    go(tree, tree.root, present, &mut out);
    out
}

// ANCHOR: main
fn main() {
    // A small "program" syntax tree. fn bar holds the bug; the other functions
    // are noise. The top level is wide so coarse pruning has real work to do.
    //
    //   program
    //   ├─ fn bar { stmt; if guard { stmt; crash(); } stmt; }  (the bug)
    //   ├─ fn f2 { stmt; stmt; }                               (irrelevant)
    //   ├─ fn f3 { stmt; stmt; }                               (irrelevant)
    //   ├─ fn f4 { stmt; stmt; }                               (irrelevant)
    //   ├─ fn f5 { stmt; stmt; }                               (irrelevant)
    //   └─ fn f6 { stmt; stmt; }                               (irrelevant)
    let tree = std::rc::Rc::new(example_tree());
    // The configuration is the leaves only -- the atomic content of the program.
    let all: Configuration = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.children.is_empty())
        .map(|(&id, _)| id)
        .collect();

    // Interesting iff the program still contains the crash() call (leaf 21).
    // Keeping that one leaf keeps its whole ancestor chain in the rendered tree:
    // program → fn bar → if → crash().
    const CRASH: AtomicUnit = 21;
    let expected: Vec<AtomicUnit> = vec![21]; // just the crash() leaf

    for (name, run) in [
        ("HDD + DDMin", 0u8),
        ("HDD + ProbDD", 1u8),
    ] {
        println!("\n==================  {name}  ==================");
        let calls = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let counter = calls.clone();
        let otree = tree.clone();
        let oracle = move |c: &Configuration| {
            counter.set(counter.get() + 1);
            let verdict = if c.contains(&CRASH) {
                Verdict::Interesting
            } else {
                Verdict::NotInteresting
            };
            println!(
                "  test {:?}  ->  {}",
                render(&otree, c),
                if verdict == Verdict::Interesting { "crashes (keep)" } else { "ok (reject)" },
            );
            verdict
        };

        let result = if run == 0 {
            reduce(all.clone(), &oracle, Hdd { tree: &*tree, new_minimizer: || DDMin, level: 1 })
        } else {
            reduce(
                all.clone(),
                &oracle,
                Hdd {
                    tree: &*tree,
                    new_minimizer: || ProbDD { probs: HashMap::new(), p0: 0.1 },
                    level: 1,
                },
            )
        };

        let mut got: Vec<AtomicUnit> = result.iter().copied().collect();
        got.sort_unstable();
        println!("  => minimized to {}  in {} oracle calls", render(&tree, &result), calls.get());
        assert_eq!(got, expected);
    }
}

fn example_tree() -> Tree {
    fn n(label: &'static str, children: Vec<AtomicUnit>) -> Node {
        Node { label, children }
    }
    let nodes: HashMap<AtomicUnit, Node> = HashMap::from([
        (0, n("program", vec![1, 2, 3, 4, 5, 6])),
        // fn bar — holds the bug
        (1, n("fn bar", vec![7, 8, 9])),
        (7, n("stmt b1", vec![])),
        (8, n("if guard", vec![20, 21])),
        (20, n("stmt g", vec![])),
        (21, n("crash()", vec![])),
        (9, n("stmt b2", vec![])),
        // fn f2..f6 — irrelevant noise, each a small subtree
        (2, n("fn f2", vec![10, 11])),
        (10, n("stmt", vec![])),
        (11, n("stmt", vec![])),
        (3, n("fn f3", vec![12, 13])),
        (12, n("stmt", vec![])),
        (13, n("stmt", vec![])),
        (4, n("fn f4", vec![14, 15])),
        (14, n("stmt", vec![])),
        (15, n("stmt", vec![])),
        (5, n("fn f5", vec![16, 17])),
        (16, n("stmt", vec![])),
        (17, n("stmt", vec![])),
        (6, n("fn f6", vec![18, 19])),
        (18, n("stmt", vec![])),
        (19, n("stmt", vec![])),
    ]);
    Tree::new(0, nodes)
}
// ANCHOR_END: main
// ANCHOR_END: all
