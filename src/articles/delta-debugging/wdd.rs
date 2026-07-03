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

// ANCHOR: atomic-unit
/// An indivisible piece of the input: a char, token, line, etc.
/// Different inputs have different atomic units, so the framework fixes
/// no concrete type: anything copyable, hashable, and orderable serves.
trait AtomicUnit: Copy + Eq + std::hash::Hash + Ord {}
impl<T: Copy + Eq + std::hash::Hash + Ord> AtomicUnit for T {}
// ANCHOR_END: atomic-unit

/// This chapter's atomic unit: a token of the program, identified by
/// its position in source order (0, 1, 2, ...).
type Token = u32;

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
                .map(move |d| config - &d); // First every δ = ∇ᵢ (keep only Δᵢ), then every δ = Δᵢ (drop Δᵢ).
            keep_only.chain(subsets)
        })
        .filter(|delta| !delta.is_empty())
    }
}
// ANCHOR_END: ddmin

// ANCHOR: weighted-partition
/// Split `config` into at most `n` chunks of roughly equal *total weight*
/// (compare `partition`, which makes chunks of roughly equal *count*).
fn weighted_partition<U: AtomicUnit>(
    config: &Configuration<U>,
    n: usize,
    unit2weight: &HashMap<U, u64>,
) -> Vec<Delta<U>> {
    let mut items: Vec<U> =
        config.iter().copied().collect();
    items.sort_unstable(); // deterministic chunks for a reproducible demo
    let len = items.len();
    if n == 0 || len == 0 {
        return Vec::new();
    }
    if n >= len {
        // finest granularity: every element on its own
        return items
            .into_iter()
            .map(|u| Delta::<U>::from([u]))
            .collect();
    }
    let total: u64 = items.iter().map(|u| unit2weight[u]).sum();
    let share = total as f64 / n as f64; // target weight per chunk

    let mut chunks: Vec<Delta<U>> = Vec::new();
    let mut cur = Delta::new();
    let mut acc = 0u64;
    for &u in &items {
        let w = unit2weight[&u];
        // When cur is nonempty and there is room for more chunks,
        // check if the current chunk is closer to the target weight
        // than it would be if we added this unit.
        if !cur.is_empty()
            && chunks.len() < n - 1
            && (acc as f64 - share).abs()
                <= (acc as f64 + w as f64 - share).abs()
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
struct Wdd<'w, U: AtomicUnit> {
    unit2weight: &'w HashMap<U, u64>,
}

impl<U: AtomicUnit> Policy<U> for Wdd<'_, U> {
    fn propose(
        &mut self,
        config: &Configuration<U>,
    ) -> impl Iterator<Item = Delta<U>> {
        let units = config.len();
        let unit2weight = self.unit2weight;

        // The same n = 2, 4, 8, ... escalation as DDMin ...
        successors(Some(2), move |&n| {
            (n < units).then(|| (2 * n).min(units))
        })
        .flat_map(move |n| {
            // ... only the partitioning is weight-aware.
            let subsets =
                weighted_partition(config, n, unit2weight);
            let keep_only = subsets
                .clone()
                .into_iter()
                .map(move |d| config - &d);
            keep_only.chain(subsets)
        })
        // a 1-element split's "keep everything" complement is empty; never propose it
        .filter(|delta| !delta.is_empty())
    }
}
// ANCHOR_END: wdd

// ANCHOR: tree
/// Identifies a node of the parse tree. *Not* an atomic unit: internal
/// nodes never appear in a Configuration. A leaf (token) node corresponds
/// to exactly one atomic unit: its source-order index, `tree.leaf2token[&id]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct NodeId(u32);

struct Node {
    label: &'static str,
    children: Vec<NodeId>,
}

struct Tree {
    id2node: HashMap<NodeId, Node>,
    root: NodeId,
    node2depth: HashMap<NodeId, usize>, // BFS depth of each node
    node2parent: HashMap<NodeId, NodeId>,
    max_depth: usize,
    leaf2token: HashMap<NodeId, Token>, // a leaf's token is its source-order index
    token2leaf: HashMap<Token, NodeId>, // inverse of `leaf2token`
}
// ANCHOR_END: tree

impl Tree {
    fn new(
        root: NodeId,
        id2node: HashMap<NodeId, Node>,
    ) -> Tree {
        // BFS from the root to label every node with its depth (= its level)
        // and remember its parent.
        let mut node2depth = HashMap::new();
        let mut node2parent = HashMap::new();
        let mut max_depth = 0;
        let mut frontier = vec![root];
        let mut d = 0;
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for &id in &frontier {
                node2depth.insert(id, d);
                max_depth = d;
                for &c in &id2node[&id].children {
                    node2parent.insert(c, id);
                    next.push(c);
                }
            }
            frontier = next;
            d += 1;
        }
        // DFS in child order (= source order): the k-th leaf from the left
        // is the token with source-order index k, i.e. atomic unit k.
        let mut leaf2token = HashMap::new();
        let mut token2leaf = HashMap::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = &id2node[&id];
            if node.children.is_empty() {
                let u = token2leaf.len() as Token;
                leaf2token.insert(id, u);
                token2leaf.insert(u, id);
            } else {
                // push in reverse so children pop left-to-right
                stack.extend(
                    node.children.iter().rev().copied(),
                );
            }
        }
        Tree {
            id2node,
            root,
            node2depth,
            node2parent,
            max_depth,
            leaf2token,
            token2leaf,
        }
    }

    // ANCHOR: leaves-under
    /// Node space -> unit space: the surviving atomic units in the subtree
    /// rooted at `id`. This is how dropping a node becomes a removal of
    /// atomic units the `reduce` loop can test.
    fn leaves_under(
        &self,
        id: NodeId,
        present: &Configuration<Token>,
    ) -> Delta<Token> {
        let mut out = Delta::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = &self.id2node[&n];
            if node.children.is_empty() {
                let u = self.leaf2token[&n];
                if present.contains(&u) {
                    out.insert(u);
                }
            } else {
                stack.extend(node.children.iter().copied());
            }
        }
        out
    }
    // ANCHOR_END: leaves-under

    // ANCHOR: alive-level
    /// Unit space -> node space: the level-`level` subtrees that still hold
    /// a surviving token, found by walking each survivor's leaf node up to
    /// its ancestor at that level.
    fn alive_level_nodes(
        &self,
        level: usize,
        present: &Configuration<Token>,
    ) -> Configuration<NodeId> {
        present
            .iter()
            .map(|&u| self.token2leaf[&u])
            .filter(|leaf| self.node2depth[leaf] >= level)
            .map(|leaf| self.ancestor_at(leaf, level))
            .collect()
    }

    /// Walk up from `id` to its ancestor sitting at `level`.
    fn ancestor_at(
        &self,
        mut id: NodeId,
        level: usize,
    ) -> NodeId {
        while self.node2depth[&id] > level {
            id = self.node2parent[&id];
        }
        id
    }
    // ANCHOR_END: alive-level


    // ANCHOR: weight
    /// The weight of every node: how many tokens (atomic units) its subtree
    /// ultimately contains. Weights live in *node space* -- they describe
    /// subtrees, the things HDD's inner minimizer picks among.
    fn leaf_counts(&self) -> HashMap<NodeId, u64> {
        fn go(
            tree: &Tree,
            id: NodeId,
            out: &mut HashMap<NodeId, u64>,
        ) -> u64 {
            let node = &tree.id2node[&id];
            let count = if node.children.is_empty() {
                1
            } else {
                node.children
                    .iter()
                    .map(|&c| go(tree, c, out))
                    .sum()
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
/// HDD is a Policy over atomic units. It walks the tree level by level
/// (coarse → fine) and, for the current level, lets an *inner policy*
/// -- a `Policy<NodeId>` -- choose which of that level's subtrees to
/// drop. Each chosen node is mapped down to the units under it before
/// the single `reduce` loop tests the removal.
struct Hdd<'t, F, P> {
    tree: &'t Tree,
    new_minimizer: F, // build a fresh list-minimizer for a level, e.g. `|| DDMin`
    level: usize, // the shallowest level not yet known to be minimal
    minimizer: Option<P>, // The inner minimizer for the current level
    level_subtrees: Configuration<NodeId>, // a field, not a local, so `propose`'s returned iterator can borrow it
}

impl<'t, F, P> Hdd<'t, F, P>
where
    F: Fn() -> P,
    P: Policy<NodeId>,
{
    fn new(
        tree: &'t Tree,
        level: usize,
        new_minimizer: F,
    ) -> Self {
        Hdd {
            tree,
            new_minimizer,
            level,
            minimizer: None,
            level_subtrees: Configuration::new(),
        }
    }
}

// To the `reduce` loop HDD removes atomic units (`Policy` defaults to
// `Policy<AtomicUnit>`); its inner minimizer picks among *subtrees*.
impl<'t, F, P> Policy<Token> for Hdd<'t, F, P>
where
    F: Fn() -> P,
    P: Policy<NodeId>,
{
    fn propose(
        &mut self,
        config: &Configuration<Token>,
    ) -> impl Iterator<Item = Delta<Token>> {
        let tree = self.tree;
        let level = self.level;

        // Build this level's minimizer on its first pass.
        if self.minimizer.is_none() {
            self.minimizer = Some((self.new_minimizer)());
        }

        // Hand the inner minimizer the level's *subtrees*
        self.level_subtrees =
            tree.alive_level_nodes(level, config);
        let subtrees = &self.level_subtrees;
        let minimizer = self.minimizer.as_mut().unwrap();
        // Lazily: `reduce` stops pulling at the first success, so a stateful
        // inner policy only ever advances its model over *confirmed* failures.
        minimizer.propose(subtrees).map(
            move |drop| -> Delta<Token> {
                // dropping a subtree drops the units under it
                drop.iter()
                    .flat_map(|&id| {
                        tree.leaves_under(id, config)
                    })
                    .collect()
            },
        )
    }

    fn on_reduced(
        &mut self,
        reduced: Option<&Configuration<Token>>,
    ) -> bool {
        let (tree, level) = (self.tree, self.level);
        // Report the pass outcome to the inner minimizer
        // in its own space: this level's still-live
        // subtrees, or `None` when nothing was found.
        // Driving the inner policy through its full
        // protocol keeps HDD agnostic to the inner policy.
        let subtrees = reduced
            .map(|c| tree.alive_level_nodes(level, c));
        let inner = self.minimizer.as_mut().unwrap();
        if inner.on_reduced(subtrees.as_ref()) {
            return true; // inner isn't minimal here yet
        }
        // The inner policy is minimal: descend and rebuild.
        self.level += 1;
        self.minimizer = None;
        self.level <= tree.max_depth
    }
}
// ANCHOR_END: hdd

/// Render a configuration (a set of surviving tokens) back into nested
/// source-ish text: a node is shown iff some token under it survives.
fn render(tree: &Tree, present: &Configuration<Token>) -> String {
    fn alive(
        tree: &Tree,
        id: NodeId,
        present: &Configuration<Token>,
    ) -> bool {
        let node = &tree.id2node[&id];
        if node.children.is_empty() {
            present.contains(&tree.leaf2token[&id])
        } else {
            node.children
                .iter()
                .any(|&c| alive(tree, c, present))
        }
    }
    fn go(
        tree: &Tree,
        id: NodeId,
        present: &Configuration<Token>,
        out: &mut String,
    ) {
        let node = &tree.id2node[&id];
        out.push_str(node.label);
        let kids: Vec<NodeId> = node
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
    // The configuration is every token of the program: units 0..n in
    // source order. Tree nodes are bookkeeping; they never sit in it.
    let all: Configuration<Token> =
        (0..tree.token2leaf.len() as Token).collect();
    // Each node's weight = how many tokens it contains. Precompute once.
    let unit2weight = tree.leaf_counts();

    // Interesting iff the program still contains the crash() token.
    let crash: Token = tree
        .id2node
        .iter()
        .find(|(_, n)| n.label == "crash()")
        .map(|(id, _)| tree.leaf2token[id])
        .unwrap();
    let expected = vec![crash]; // just the crash() token

    for (name, run) in [
        ("HDD + DDMin (partition by count) ", 0u8),
        ("HDD + WDD   (partition by weight)", 1u8),
    ] {
        println!("\n==================  {name}  ==================");
        let calls =
            std::rc::Rc::new(std::cell::Cell::new(0u32));
        let counter = calls.clone();
        let otree = tree.clone();
        let oracle = move |c: &Configuration<Token>| {
            counter.set(counter.get() + 1);
            let verdict = if c.contains(&crash) {
                Verdict::Interesting
            } else {
                Verdict::NotInteresting
            };
            println!(
                "  test {:?}  ->  {}",
                render(&otree, c),
                if verdict == Verdict::Interesting {
                    "crashes (keep)"
                } else {
                    "ok (reject)"
                },
            );
            verdict
        };

        // Start at level 1: level 0 holds only the root, and deleting the
        // whole program can never stay interesting.
        let result = if run == 0 {
            reduce(
                all.clone(),
                &oracle,
                Hdd::new(&*tree, 1, || DDMin),
            )
        } else {
            reduce(
                all.clone(),
                &oracle,
                Hdd::new(&*tree, 1, || Wdd {
                    unit2weight: &unit2weight,
                }),
            )
        };

        let mut got: Vec<Token> =
            result.iter().copied().collect();
        got.sort_unstable();
        println!(
            "  => minimized to {}  in {} oracle calls",
            render(&tree, &result),
            calls.get()
        );
        assert_eq!(got, expected);
        assert_eq!(calls.get(), if run == 0 { 13 } else { 8 });
    }
}

fn example_tree() -> Tree {
    fn n(
        label: &'static str,
        children: Vec<u32>,
    ) -> Node {
        Node {
            label,
            children: children
                .into_iter()
                .map(NodeId)
                .collect(),
        }
    }
    let id2node: HashMap<NodeId, Node> = HashMap::from([
        (NodeId(0), n("program", vec![1, 2, 3, 4, 5, 6, 7])),
        // fn big — the heavy subtree that holds the bug
        (NodeId(1), n("fn big", vec![8, 20, 21, 22, 23])),
        (NodeId(8), n("block", vec![9, 30, 31, 32])),
        (NodeId(9), n("if", vec![40, 11])),
        (NodeId(40), n("guard", vec![])),
        (NodeId(11), n("crash()", vec![])),
        (NodeId(20), n("stmt", vec![])),
        (NodeId(21), n("stmt", vec![])),
        (NodeId(22), n("stmt", vec![])),
        (NodeId(23), n("stmt", vec![])),
        (NodeId(30), n("stmt", vec![])),
        (NodeId(31), n("stmt", vec![])),
        (NodeId(32), n("stmt", vec![])),
        // fn n2..n7 — six tiny noise functions, one leaf each
        (NodeId(2), n("fn n2", vec![])),
        (NodeId(3), n("fn n3", vec![])),
        (NodeId(4), n("fn n4", vec![])),
        (NodeId(5), n("fn n5", vec![])),
        (NodeId(6), n("fn n6", vec![])),
        (NodeId(7), n("fn n7", vec![])),
    ]);
    Tree::new(NodeId(0), id2node)
}
// ANCHOR_END: main
// ANCHOR_END: all
