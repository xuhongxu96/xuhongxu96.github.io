// Weighted delta debugging: WDD is DDMin with one change -- it partitions a list
// by *weight* (fragment size) instead of by count. Here it is plugged into HDD as
// the per-level minimizer, where each level's subtrees have natural weights (leaf
// counts). The framework, tree, and HDD are the same as the HDD page. Compiles
// and runs on its own:
//
//     rustc --edition 2024 wdd.rs && ./wdd

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

// ANCHOR: weighted-partition
/// Split `config` into at most `n` chunks of roughly equal *total weight*
/// (compare `partition`, which makes chunks of roughly equal *count*).
fn weighted_partition(
    config: &Configuration,
    n: usize,
    weight: &HashMap<AtomicUnit, u64>,
) -> Vec<Delta> {
    let mut items: Vec<AtomicUnit> = config.iter().copied().collect();
    items.sort_unstable(); // deterministic chunks for a reproducible demo
    let len = items.len();
    if n == 0 || len == 0 {
        return Vec::new();
    }
    if n >= len {
        // finest granularity: every element on its own
        return items.into_iter().map(|u| Delta::from([u])).collect();
    }
    let total: u64 = items.iter().map(|u| weight[u]).sum();
    let share = total as f64 / n as f64; // target weight per chunk

    let mut chunks: Vec<Delta> = Vec::new();
    let mut cur = Delta::new();
    let mut acc = 0u64;
    for &u in &items {
        let w = weight[&u];
        // When cur is nonempty and there is room for more chunks,
        // check if the current chunk is closer to the target weight
        // than it would be if we added this unit.
        if !cur.is_empty()
            && chunks.len() < n - 1
            && (acc as f64 - share).abs() <= (acc as f64 + w as f64 - share).abs()
        {
            // If so, save the current chunk and start a new chunk
            chunks.push(std::mem::take(&mut cur));
            acc = 0;
        }
        // Otherwise, add the unit to the current chunk and keep going.
        cur.insert(u);
        acc += w;
    }
    chunks.push(cur);
    chunks
}
// ANCHOR_END: weighted-partition

// ANCHOR: wdd
/// WDD is DDMin with a single change: it partitions by *weight*, not by count.
struct Wdd<'w> {
    weight: &'w HashMap<AtomicUnit, u64>,
}

impl Policy for Wdd<'_> {
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        let units = config.len();
        let weight = self.weight;

        // The same n = 2, 4, 8, ... escalation as DDMin ...
        successors(Some(2), move |&n| (n < units).then(|| (2 * n).min(units)))
            .flat_map(move |n| {
                // ... only the partitioning is weight-aware.
                let subsets = weighted_partition(config, n, weight);
                let keep_only = subsets.clone().into_iter().map(move |d| config - &d);
                keep_only.chain(subsets)
            })
    }
}
// ANCHOR_END: wdd

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

    // ANCHOR: weight
    /// The weight of every node: how many leaves it ultimately contains.
    fn leaf_counts(&self) -> HashMap<AtomicUnit, u64> {
        fn go(tree: &Tree, id: AtomicUnit, out: &mut HashMap<AtomicUnit, u64>) -> u64 {
            let node = &tree.nodes[&id];
            let count = if node.children.is_empty() {
                1
            } else {
                node.children.iter().map(|&c| go(tree, c, out)).sum()
            };
            out.insert(id, count);
            count
        }
        let mut out = HashMap::new();
        go(self, self.root, &mut out);
        out
    }
    // ANCHOR_END: weight
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
    // A "program" tree with very *uneven* subtree sizes. `fn big` holds the bug
    // and is large; every other function/statement is tiny noise. At each level
    // HDD hands the surviving subtrees to a list-minimizer -- and one of them is
    // far heavier than the rest:
    //
    //   program                                level 1: big(9) vs six 1-leaf fns
    //   ├─ fn big { ...; block { ...; if guard { crash() } } }   (the bug, heavy)
    //   └─ fn n2 .. fn n7                       (six tiny noise functions)
    let tree = std::rc::Rc::new(example_tree());
    // The configuration is the leaves only -- the atomic content of the program.
    let all: Configuration = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.children.is_empty())
        .map(|(&id, _)| id)
        .collect();
    // Each node's weight = how many leaves it contains. Precompute once.
    let weight = tree.leaf_counts();

    // Interesting iff the program still contains the crash() call (leaf 11).
    const CRASH: AtomicUnit = 11;
    let expected: Vec<AtomicUnit> = vec![11]; // just the crash() leaf

    for (name, run) in [
        ("HDD + DDMin (partition by count) ", 0u8),
        ("HDD + WDD   (partition by weight)", 1u8),
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
                    new_minimizer: || Wdd { weight: &weight },
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
        (0, n("program", vec![1, 2, 3, 4, 5, 6, 7])),
        // fn big — the heavy subtree that holds the bug
        (1, n("fn big", vec![8, 20, 21, 22, 23])),
        (8, n("block", vec![9, 30, 31, 32])),
        (9, n("if", vec![40, 11])),
        (40, n("guard", vec![])),
        (11, n("crash()", vec![])),
        (20, n("stmt", vec![])),
        (21, n("stmt", vec![])),
        (22, n("stmt", vec![])),
        (23, n("stmt", vec![])),
        (30, n("stmt", vec![])),
        (31, n("stmt", vec![])),
        (32, n("stmt", vec![])),
        // fn n2..n7 — six tiny noise functions, one leaf each
        (2, n("fn n2", vec![])),
        (3, n("fn n3", vec![])),
        (4, n("fn n4", vec![])),
        (5, n("fn n5", vec![])),
        (6, n("fn n6", vec![])),
        (7, n("fn n7", vec![])),
    ]);
    Tree::new(0, nodes)
}
// ANCHOR_END: main
// ANCHOR_END: all
