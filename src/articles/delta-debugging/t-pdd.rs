// T-PDD: a probabilistic policy over the *whole* parse tree at once. Where HDD
// hands each level a fresh, memoryless list-minimizer, and Perses trades pure
// deletion for node replacement, T-PDD keeps one Bayesian belief per node for
// the entire run and always tests the single candidate -- at any depth -- with
// the highest expected number of leaves removed. Reuses Perses's parse tree
// (Kind, Node, Tree, live, leaves_under) and ProbDD's update rule verbatim.
// Compiles and runs on its own:
//
//     rustc --edition 2024 t-pdd.rs && ./t-pdd

// ANCHOR: all
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::successors;

/// An indivisible piece of the input: here, a node of the parse tree.
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
trait Policy {
    /// Generate candidate removal sets *lazily*.
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta>;

    /// React to a reduction pass.
    fn on_reduced(
        &mut self,
        reduced: Option<&Configuration>,
    ) -> bool {
        reduced.is_some()
    }
}
// ANCHOR_END: policy

fn partition(
    config: &Configuration,
    n: usize,
) -> Vec<Delta> {
    let mut items: Vec<AtomicUnit> =
        config.iter().copied().collect();
    items.sort_unstable();
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

struct DDMin;

impl Policy for DDMin {
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        let units = config.len();
        successors(Some(2), move |&n| {
            (n < units).then(|| (2 * n).min(units))
        })
        .flat_map(move |n| {
            let subsets = partition(config, n);
            let keep_only = subsets
                .clone()
                .into_iter()
                .map(move |d| config - &d);
            keep_only.chain(subsets)
        })
        .filter(|delta| !delta.is_empty())
    }
}

// ProbDD, reused verbatim from probdd.rs/hdd.rs -- needed only as the fair
// "probabilistic policy" baseline compared against in `main`, not re-taught.
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

fn best_prefix(
    probs: &HashMap<AtomicUnit, f64>,
) -> Vec<AtomicUnit> {
    let mut units: Vec<AtomicUnit> =
        probs.keys().copied().collect();
    units.sort_by(|a, b| {
        probs[a]
            .partial_cmp(&probs[b])
            .unwrap()
            .then(a.cmp(b))
    });
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

fn bayes_update(
    probs: &mut HashMap<AtomicUnit, f64>,
    pre: &[AtomicUnit],
) {
    let survive: f64 =
        pre.iter().map(|u| 1.0 - probs[u]).product();
    let denom = 1.0 - survive;
    if denom <= 0.0 {
        return;
    }
    for u in pre {
        let p = probs[u];
        probs.insert(*u, (p / denom).min(1.0));
    }
}

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

// ANCHOR: cst
/// A node's grammar kind. `Token` is a terminal leaf; `List` is a Kleene node
/// (zero-or-more, so its children are deletable); the rest are non-terminals.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Token,
    List,
    Func,
    Block,
    Call,
}

fn is_stmt(kind: Kind) -> bool {
    matches!(kind, Kind::Block | Kind::Call)
}

struct Node {
    kind: Kind,
    label: &'static str,
    children: Vec<AtomicUnit>,
}

struct Tree {
    nodes: HashMap<AtomicUnit, Node>,
    root: AtomicUnit,
    depth: HashMap<AtomicUnit, usize>,
    parent: HashMap<AtomicUnit, AtomicUnit>,
    max_depth: usize,
}
// ANCHOR_END: cst

impl Tree {
    fn new(
        root: AtomicUnit,
        nodes: HashMap<AtomicUnit, Node>,
    ) -> Tree {
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
        Tree {
            nodes,
            root,
            depth,
            parent,
            max_depth,
        }
    }

    // ANCHOR: tree-ops
    fn leaves_under(
        &self,
        id: AtomicUnit,
        present: &Configuration,
    ) -> Delta {
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

    fn present(
        &self,
        id: AtomicUnit,
        config: &Configuration,
    ) -> bool {
        !self.leaves_under(id, config).is_empty()
    }

    fn descendants(
        &self,
        id: AtomicUnit,
        config: &Configuration,
    ) -> Vec<AtomicUnit> {
        let mut out = Vec::new();
        let mut stack: Vec<AtomicUnit> = self.nodes[&id]
            .children
            .iter()
            .copied()
            .collect();
        while let Some(n) = stack.pop() {
            if !self.present(n, config) {
                continue;
            }
            out.push(n);
            stack.extend(
                self.nodes[&n].children.iter().copied(),
            );
        }
        out
    }

    // ANCHOR: alive
    fn alive_level_nodes(
        &self,
        level: usize,
        present: &Configuration,
    ) -> Configuration {
        present
            .iter()
            .copied()
            .filter(|leaf| self.depth[leaf] >= level)
            .map(|leaf| self.ancestor_at(leaf, level))
            .filter(|&node| self.deletable(node))
            .collect()
    }
    // ANCHOR_END: alive

    // ANCHOR: deletable
    fn deletable(&self, id: AtomicUnit) -> bool {
        self.parent.get(&id).is_some_and(|p| {
            self.nodes[p].kind == Kind::List
        })
    }
    // ANCHOR_END: deletable

    fn ancestor_at(
        &self,
        mut id: AtomicUnit,
        level: usize,
    ) -> AtomicUnit {
        while self.depth[&id] > level {
            id = self.parent[&id];
        }
        id
    }
    // ANCHOR_END: tree-ops

    // ANCHOR: subsume
    fn can_replace(
        &self,
        n: AtomicUnit,
        d: AtomicUnit,
        config: &Configuration,
    ) -> bool {
        if n == d {
            return false;
        }
        // `n`'s own parent may no longer be *live* -- an earlier replacement
        // can strip a mandatory wrapper several levels up, leaving `n` as the
        // sole surviving content standing in for some ancestor further out (a
        // replacement's delta always empties every sibling in the replaced
        // node's scope, so this really is "sole"). Walk up through that dead
        // chain to the ancestor `n` is actually filling in for: the first
        // node whose own parent is live (or the root).
        let mut anchor = n;
        while let Some(&p) = self.parent.get(&anchor) {
            if p == self.root || self.live(p, config) {
                break;
            }
            anchor = p;
        }
        let d_kind = self.nodes[&d].kind;
        match self.parent.get(&anchor) {
            Some(p) if self.nodes[p].kind == Kind::List => {
                is_stmt(d_kind)
            }
            Some(_) => d_kind == self.nodes[&anchor].kind,
            None => false,
        }
    }
    // ANCHOR_END: subsume

    // ANCHOR: live
    fn live(
        &self,
        id: AtomicUnit,
        config: &Configuration,
    ) -> bool {
        let node = &self.nodes[&id];
        match node.kind {
            Kind::Token => config.contains(&id),
            Kind::List => self
                .parent
                .get(&id)
                .is_some_and(|&p| self.live(p, config)),
            _ => node
                .children
                .iter()
                .filter(|&&c| {
                    self.nodes[&c].kind != Kind::List
                })
                .all(|&c| self.live(c, config)),
        }
    }
    // ANCHOR_END: live
}

// ANCHOR: render
fn render(tree: &Tree, present: &Configuration) -> String {
    fn go(
        tree: &Tree,
        id: AtomicUnit,
        present: &Configuration,
        out: &mut String,
    ) {
        let node = &tree.nodes[&id];
        if node.children.is_empty() {
            if present.contains(&id) {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(node.label);
            }
        } else {
            for &c in &node.children {
                go(tree, c, present, out);
            }
        }
    }
    let mut out = String::new();
    go(tree, tree.root, present, &mut out);
    out
}
// ANCHOR_END: render

// ANCHOR: hdd
struct Hdd<'t, F, P> {
    tree: &'t Tree,
    new_minimizer: F,
    level: usize,
    minimizer: Option<P>, // The inner minimizer for the current level
    level_subtrees: Configuration, // a field, not a local, so `propose`'s returned iterator can borrow it
}

impl<'t, F, P> Hdd<'t, F, P>
where
    F: Fn() -> P,
    P: Policy,
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

impl<'t, F, P> Policy for Hdd<'t, F, P>
where
    F: Fn() -> P,
    P: Policy,
{
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        let tree = self.tree;
        let level = self.level;
        // Build this level's minimizer on its first pass. `on_reduced` clears it
        // only when we descend a level, so a stateful inner policy (ProbDD) keeps
        // learning across a level's passes and is reset only at a level boundary.
        if self.minimizer.is_none() {
            self.minimizer = Some((self.new_minimizer)());
        }
        self.level_subtrees =
            tree.alive_level_nodes(level, config);
        let subtrees = &self.level_subtrees;
        let minimizer = self.minimizer.as_mut().unwrap();
        // Lazily: `reduce` stops pulling at the first success, so a stateful
        // inner policy only ever advances its model over *confirmed* failures.
        minimizer.propose(subtrees).map(
            move |drop| -> Delta {
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
        reduced: Option<&Configuration>,
    ) -> bool {
        if reduced.is_some() {
            return true; // progress at this level; keep going
        }
        // A whole `propose` stream failed: this level is minimal, so descend and
        // rebuild the minimizer for the next one.
        self.level += 1;
        self.minimizer = None;
        self.level <= self.tree.max_depth
    }
}
// ANCHOR_END: hdd

// ANCHOR: perses
struct Perses<'t, F> {
    tree: &'t Tree,
    new_minimizer: F,
}

impl<F, P> Policy for Perses<'_, F>
where
    F: Fn() -> P,
    P: Policy,
{
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        let tree = self.tree;
        let size: HashMap<AtomicUnit, usize> = tree
            .nodes
            .keys()
            .map(|&id| {
                (id, tree.leaves_under(id, config).len())
            })
            .collect();

        let mut nodes: Vec<AtomicUnit> = tree
            .nodes
            .keys()
            .copied()
            .filter(|&id| {
                !tree.nodes[&id].children.is_empty()
                    && tree.live(id, config)
            })
            .collect();
        nodes.sort_by(|&a, &b| {
            size[&b].cmp(&size[&a]).then(a.cmp(&b))
        });

        let mut cands: Vec<Delta> = Vec::new();
        for n in nodes {
            let n_leaves = tree.leaves_under(n, config);

            let mut ds: Vec<AtomicUnit> = tree
                .descendants(n, config)
                .into_iter()
                .filter(|&d| {
                    tree.live(d, config)
                        && tree.can_replace(n, d, config)
                })
                .collect();
            ds.sort_by(|&a, &b| {
                size[&a].cmp(&size[&b]).then(a.cmp(&b))
            });
            for d in ds {
                let delta: Delta = n_leaves
                    .difference(
                        &tree.leaves_under(d, config),
                    )
                    .copied()
                    .collect();
                if !delta.is_empty() {
                    cands.push(delta);
                }
            }

            if tree.nodes[&n].kind == Kind::List {
                let elems: Configuration = tree.nodes[&n]
                    .children
                    .iter()
                    .copied()
                    .filter(|&c| tree.present(c, config))
                    .collect();
                let mut minimizer = (self.new_minimizer)();
                for drop in minimizer.propose(&elems) {
                    cands.push(
                        drop.iter()
                            .flat_map(|&id| {
                                tree.leaves_under(
                                    id, config,
                                )
                            })
                            .collect(),
                    );
                }
            }
        }
        cands.into_iter()
    }
}
// ANCHOR_END: perses

// ANCHOR: valid
fn parses(src: &str) -> bool {
    fn is_ident(t: &str) -> bool {
        !matches!(
            t,
            "int"
                | "main"
                | "if"
                | "("
                | ")"
                | "{"
                | "}"
                | ";"
        )
    }
    struct Parser<'a> {
        toks: Vec<&'a str>,
        pos: usize,
    }
    impl Parser<'_> {
        fn eat(&mut self, s: &str) -> bool {
            let ok = self.toks.get(self.pos) == Some(&s);
            self.pos += ok as usize;
            ok
        }
        fn ident(&mut self) -> bool {
            let ok = self
                .toks
                .get(self.pos)
                .is_some_and(|t| is_ident(t));
            self.pos += ok as usize;
            ok
        }
        fn func(&mut self) -> bool {
            self.eat("int")
                && self.eat("main")
                && self.eat("(")
                && self.eat(")")
                && self.block()
        }
        fn block(&mut self) -> bool {
            if !self.eat("{") {
                return false;
            }
            while self.stmt() {}
            self.eat("}")
        }
        fn stmt(&mut self) -> bool {
            let save = self.pos;
            self.if_stmt()
                || (self.reset(save), self.block()).1
                || (self.reset(save), self.call()).1
                || (self.reset(save), false).1
        }
        fn if_stmt(&mut self) -> bool {
            self.eat("if")
                && self.eat("(")
                && self.ident()
                && self.eat(")")
                && self.block()
        }
        fn call(&mut self) -> bool {
            self.ident()
                && self.eat("(")
                && self.eat(")")
                && self.eat(";")
        }
        fn reset(&mut self, pos: usize) {
            self.pos = pos;
        }
    }
    let mut p = Parser {
        toks: src.split_whitespace().collect(),
        pos: 0,
    };
    p.func() && p.pos == p.toks.len()
}
// ANCHOR_END: valid

// ANCHOR: model
/// T-PDD's belief: `p[n]` is the conditional retention probability of `n`,
/// `P(n survives | n's parent survives)`.
struct TPdd<'t> {
    tree: &'t Tree,
    p: HashMap<AtomicUnit, f64>,
}

impl<'t> TPdd<'t> {
    /// `sigma` is the paper's one hyperparameter: the prior for a `List`
    /// element (e.g. 0.5). Everything else starts certain to survive.
    fn new(tree: &'t Tree, sigma: f64) -> TPdd<'t> {
        TPdd {
            tree,
            p: priors(tree, sigma),
        }
    }
}
// ANCHOR_END: model

// ANCHOR: priors
/// A static prior from the tree's shape:
/// a `List` element is optional (`sigma`),
/// everything else is mandatory (`1.0`).
fn priors(
    tree: &Tree,
    sigma: f64,
) -> HashMap<AtomicUnit, f64> {
    tree.nodes
        .keys()
        .copied()
        .filter(|&n| n != tree.root)
        .map(|n| {
            (n, if tree.deletable(n) { sigma } else { 1.0 })
        })
        .collect()
}
// ANCHOR_END: priors

// ANCHOR: q
/// Bottom-up within `d`'s own subtree: the model's belief that `n`'s
/// subtree ends up contributing no surviving leaf at all -- either `n`
/// itself is gone, or it survives but every one of its live children does.
fn q(
    tree: &Tree,
    p: &HashMap<AtomicUnit, f64>,
    config: &Configuration,
    n: AtomicUnit,
) -> f64 {
    let node = &tree.nodes[&n];
    let pn = p[&n];
    if node.children.is_empty() {
        return 1.0 - pn;
    }
    let product: f64 = node
        .children
        .iter()
        .copied()
        .filter(|&c| tree.live(c, config))
        .map(|c| q(tree, p, config, c))
        .product();
    (1.0 - pn) + pn * product
}
// ANCHOR_END: q

// ANCHOR: pass-prob
/// Fold `q(d)` up through `d`'s ancestors.
fn pass_prob(
    tree: &Tree,
    p: &HashMap<AtomicUnit, f64>,
    config: &Configuration,
    d: AtomicUnit,
) -> f64 {
    let mut result = q(tree, p, config, d);
    let mut node = d;
    while let Some(&a) = tree.parent.get(&node) {
        match p.get(&a) {
            Some(&pa) => result = (1.0 - pa) + pa * result,
            None => break, // `a` is the root: it has no entry in `p`
        }
        node = a;
    }
    result
}
// ANCHOR_END: pass-prob

// ANCHOR: expected-gain
/// The expected number of leaves a deletion at `d` removes, discounted by
/// the model's belief that the test would still pass.
fn expected_gain(
    tree: &Tree,
    p: &HashMap<AtomicUnit, f64>,
    config: &Configuration,
    d: AtomicUnit,
) -> f64 {
    tree.leaves_under(d, config).len() as f64
        * pass_prob(tree, p, config, d)
}
// ANCHOR_END: expected-gain

// ANCHOR: choose
/// Below this expected gain, T-PDD gives up.
const MIN_GAIN: f64 = 0.0;

/// The live node with the highest expected gain, root excluded -- the root
/// has no parent to condition on, so it has no entry in `p` at all.
/// Mandatory nodes need no special case: their expected gain is always `0`
/// (see the note above), so `MIN_GAIN` already excludes them.
fn best_candidate(
    tree: &Tree,
    p: &HashMap<AtomicUnit, f64>,
    config: &Configuration,
) -> Option<AtomicUnit> {
    let mut cands: Vec<(AtomicUnit, f64)> = tree
        .nodes
        .keys()
        .copied()
        .filter(|&id| {
            id != tree.root && tree.live(id, config)
        })
        .map(|id| (id, expected_gain(tree, p, config, id)))
        .collect();

    cands.sort_by(|&(a_id, a_gain), &(b_id, b_gain)| {
        b_gain
            .partial_cmp(&a_gain)
            .unwrap()
            .then(a_id.cmp(&b_id))
    });

    cands
        .first()
        .filter(|&&(_, gain)| gain > MIN_GAIN)
        .map(|&(id, _)| id)
}
// ANCHOR_END: choose

// ANCHOR: update
/// A deletion at `d` just failed. Raise its belief with the same shape of
/// Bayesian posterior ProbDD uses -- `p / (1 - pass_prob)`.
fn update(
    tree: &Tree,
    p: &mut HashMap<AtomicUnit, f64>,
    config: &Configuration,
    d: AtomicUnit,
) {
    let survive = pass_prob(tree, p, config, d);
    let denom = 1.0 - survive;
    if denom > 0.0 {
        let pd = p[&d];
        p.insert(d, (pd / denom).min(1.0));
    }
}
// ANCHOR_END: update

// ANCHOR: propose
impl Policy for TPdd<'_> {
    fn propose(
        &mut self,
        config: &Configuration,
    ) -> impl Iterator<Item = Delta> {
        let tree = self.tree;
        let p = &mut self.p;

        // The loop only pulls the *next* delta when the previous one failed,
        // so each iteration after the first means "that candidate failed".
        let mut last: Option<AtomicUnit> = None;
        std::iter::from_fn(move || {
            if let Some(d) = last {
                update(tree, p, config, d);
            }
            let d = best_candidate(tree, p, config)?;
            last = Some(d);
            Some(tree.leaves_under(d, config))
        })
    }
}
// ANCHOR_END: propose

// ANCHOR: main
fn main() {
    let tree = std::rc::Rc::new(example_tree());
    let all: Configuration = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.children.is_empty())
        .map(|(&id, _)| id)
        .collect();

    let crash = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label == "crash")
        .map(|(&id, _)| id)
        .unwrap();
    let setup = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label == "setup")
        .map(|(&id, _)| id)
        .unwrap();

    // ANCHOR: make-oracle
    let make_oracle = || {
        let calls =
            std::rc::Rc::new(std::cell::Cell::new(0u32));
        let counter = calls.clone();
        let otree = tree.clone();
        let oracle = move |c: &Configuration| {
            counter.set(counter.get() + 1);
            let src = render(&otree, c);
            let ok = c.contains(&crash)
                && c.contains(&setup)
                && parses(&src);
            println!(
                "  test {src:?}  ->  {}",
                if ok { "keep" } else { "reject" }
            );
            if ok {
                Verdict::Interesting
            } else {
                Verdict::NotInteresting
            }
        };
        (oracle, calls)
    };
    // ANCHOR_END: make-oracle

    let (hdd_oracle, hdd_calls) = make_oracle();
    let hdd = reduce(
        all.clone(),
        &hdd_oracle,
        Hdd::new(&*tree, 0, || DDMin),
    );
    println!(
        "HDD        => {:?}  in {} calls\n",
        render(&tree, &hdd),
        hdd_calls.get()
    );

    let (hddp_oracle, hddp_calls) = make_oracle();
    let hddp = reduce(
        all.clone(),
        &hddp_oracle,
        Hdd::new(&*tree, 0, || ProbDD {
            probs: HashMap::new(),
            p0: 0.1,
        }),
    );
    println!(
        "HDD+ProbDD => {:?}  in {} calls\n",
        render(&tree, &hddp),
        hddp_calls.get()
    );

    let (perses_oracle, perses_calls) = make_oracle();
    let perses = reduce(
        all.clone(),
        &perses_oracle,
        Perses {
            tree: &*tree,
            new_minimizer: || DDMin,
        },
    );
    println!(
        "Perses     => {:?}  in {} calls\n",
        render(&tree, &perses),
        perses_calls.get()
    );

    let (tpdd_oracle, tpdd_calls) = make_oracle();
    let tpdd = reduce(
        all.clone(),
        &tpdd_oracle,
        TPdd::new(&*tree, 0.5),
    );
    println!(
        "T-PDD      => {:?}  in {} calls\n",
        render(&tree, &tpdd),
        tpdd_calls.get()
    );

    let minimal =
        "int main ( ) { crash ( ) ; setup ( ) ; }";
    assert_eq!(render(&tree, &hdd), minimal);
    assert_eq!(render(&tree, &perses), minimal);
    assert_eq!(render(&tree, &tpdd), minimal);
}
// ANCHOR_END: main

/// A tiny builder so the nested example reads top-down instead of as a giant map.
struct Builder {
    nodes: HashMap<AtomicUnit, Node>,
    next: AtomicUnit,
}
impl Builder {
    fn new() -> Builder {
        Builder {
            nodes: HashMap::new(),
            next: 0,
        }
    }
    fn add(
        &mut self,
        kind: Kind,
        label: &'static str,
        children: Vec<AtomicUnit>,
    ) -> AtomicUnit {
        let id = self.next;
        self.next += 1;
        self.nodes.insert(
            id,
            Node {
                kind,
                label,
                children,
            },
        );
        id
    }
    fn tok(&mut self, s: &'static str) -> AtomicUnit {
        self.add(Kind::Token, s, vec![])
    }
    fn call(&mut self, name: &'static str) -> AtomicUnit {
        let n = self.tok(name);
        let lp = self.tok("(");
        let rp = self.tok(")");
        let sc = self.tok(";");
        self.add(Kind::Call, "", vec![n, lp, rp, sc])
    }
    fn list(
        &mut self,
        elems: Vec<AtomicUnit>,
    ) -> AtomicUnit {
        self.add(Kind::List, "", elems)
    }
    fn block(&mut self, list: AtomicUnit) -> AtomicUnit {
        let lb = self.tok("{");
        let rb = self.tok("}");
        self.add(Kind::Block, "", vec![lb, list, rb])
    }
    fn func(&mut self, body: AtomicUnit) -> AtomicUnit {
        let t = self.tok("int");
        let m = self.tok("main");
        let lp = self.tok("(");
        let rp = self.tok(")");
        self.add(Kind::Func, "", vec![t, m, lp, rp, body])
    }
}

fn example_tree() -> Tree {
    let mut b = Builder::new();
    // the bug needs *both* crash() and its companion setup() to survive --
    // dropping either alone hides it, so no single sibling can stand in for
    // the pair. crash and setup are scattered among the noise, not adjacent
    // to each other -- see the note in "Run It" for why that matters.
    let n1 = b.call("noise1");
    let crash = b.call("crash");
    let n2 = b.call("noise2");
    let setup = b.call("setup");
    let n3 = b.call("noise3");
    let n4 = b.call("noise4");
    let l = b.list(vec![n1, crash, n2, setup, n3, n4]);
    let body = b.block(l);
    let root = b.func(body);
    Tree::new(root, b.nodes)
}
// ANCHOR_END: all
