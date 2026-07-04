// T-PDD: a probabilistic policy over the *whole* parse tree at once. Where HDD
// hands each level a fresh, memoryless list-minimizer, and Perses trades pure
// deletion for node replacement, T-PDD keeps one Bayesian belief per node for
// the entire run and always tests the single candidate -- at any depth -- with
// the highest expected number of tokens removed. Reuses Perses's parse tree
// (Kind, NodeId, Node, Tree, live, leaves_under) and ProbDD's update rule
// verbatim. Compiles and runs on its own:
//
//     rustc --edition 2024 t-pdd.rs && ./t-pdd

// ANCHOR: all
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::successors;

// ANCHOR: atomic-unit
/// An indivisible piece of the input: a char, token, line, etc.
trait AtomicUnit: Copy + Eq + std::hash::Hash + Ord {}
impl<T: Copy + Eq + std::hash::Hash + Ord> AtomicUnit for T {}
// ANCHOR_END: atomic-unit

/// This chapter's atomic unit: a token of the program, identified by
/// its position in source order (0, 1, 2, ...).
type Token = u32;

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

fn partition<U: AtomicUnit>(
    config: &Configuration<U>,
    n: usize,
) -> Vec<Delta<U>> {
    let mut items: Vec<U> =
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

impl<U: AtomicUnit> Policy<U> for DDMin {
    fn propose(
        &mut self,
        config: &Configuration<U>,
    ) -> impl Iterator<Item = Delta<U>> {
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
struct ProbDD<U: AtomicUnit> {
    unit2prob: HashMap<U, f64>,
    p0: f64,
}

impl<U: AtomicUnit> ProbDD<U> {
    fn sync(&mut self, config: &Configuration<U>) {
        self.unit2prob.retain(|u, _| config.contains(u));
        for &u in config {
            self.unit2prob.entry(u).or_insert(self.p0);
        }
    }
}

fn best_prefix<U: AtomicUnit>(
    unit2prob: &HashMap<U, f64>,
) -> Vec<U> {
    let mut units: Vec<U> =
        unit2prob.keys().copied().collect();
    units.sort_by(|a, b| {
        unit2prob[a]
            .partial_cmp(&unit2prob[b])
            .unwrap()
            .then(a.cmp(b))
    });
    let mut survive = 1.0;
    let (mut best_k, mut best_gain) = (0, 0.0);
    for (i, u) in units.iter().enumerate() {
        survive *= 1.0 - unit2prob[u];
        let gain = (i + 1) as f64 * survive;
        if gain > best_gain {
            (best_k, best_gain) = (i + 1, gain);
        }
    }
    units.truncate(best_k);
    units
}

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
        let mut last: Option<Vec<U>> = None;
        std::iter::from_fn(move || {
            if let Some(pre) = &last {
                bayes_update(unit2prob, pre);
            }
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

// ANCHOR: cst
/// A node's grammar kind. `Token` is a terminal leaf; `List` is a Kleene node
/// (zero-or-more, so its children are deletable); the rest are non-terminals.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Token, // a terminal: "if", "(", "{", "crash", ";", ...

    List, // a Kleene list of statements (its children are removable)

    Expr, // a condition
    Func, // the function definition (root)

    IfStmt, // if ( cond ) block        \
    Block, // { stmt-list }            |- these three are statements
    Call,  // name ( ) ;               /
}

fn is_stmt(kind: Kind) -> bool {
    matches!(kind, Kind::IfStmt | Kind::Block | Kind::Call)
}

/// Identifies a node of the parse tree. *Not* an atomic unit: internal
/// nodes never appear in a Configuration. A leaf (token) node corresponds
/// to exactly one atomic unit: its source-order index, `tree.leaf2token[&id]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct NodeId(u32);

struct Node {
    kind: Kind,
    label: &'static str, // the source text, for a `Token` leaf
    children: Vec<NodeId>,
}

struct Tree {
    id2node: HashMap<NodeId, Node>,
    root: NodeId,
    node2depth: HashMap<NodeId, usize>,
    node2parent: HashMap<NodeId, NodeId>,
    max_depth: usize,
    leaf2token: HashMap<NodeId, Token>, // a leaf's token is its source-order index
    token2leaf: HashMap<Token, NodeId>, // inverse of `leaf2token`
}
// ANCHOR_END: cst

impl Tree {
    fn new(
        root: NodeId,
        id2node: HashMap<NodeId, Node>,
    ) -> Tree {
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

    fn present(
        &self,
        id: NodeId,
        config: &Configuration<Token>,
    ) -> bool {
        !self.leaves_under(id, config).is_empty()
    }

    fn descendants(
        &self,
        id: NodeId,
        config: &Configuration<Token>,
    ) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.id2node[&id]
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
                self.id2node[&n].children.iter().copied(),
            );
        }
        out
    }

    // ANCHOR: alive
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
            .filter(|&node| self.deletable(node))
            .collect()
    }
    // ANCHOR_END: alive

    // ANCHOR: deletable
    fn deletable(&self, id: NodeId) -> bool {
        self.node2parent.get(&id).is_some_and(|p| {
            self.id2node[p].kind == Kind::List
        })
    }
    // ANCHOR_END: deletable

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


    // ANCHOR: live
    fn live(
        &self,
        id: NodeId,
        config: &Configuration<Token>,
    ) -> bool {
        let node = &self.id2node[&id];
        match node.kind {
            Kind::Token => {
                config.contains(&self.leaf2token[&id])
            }
            Kind::List => self
                .node2parent
                .get(&id)
                .is_some_and(|&p| self.live(p, config)),
            _ => node
                .children
                .iter()
                .filter(|&&c| {
                    self.id2node[&c].kind != Kind::List
                })
                .all(|&c| self.live(c, config)),
        }
    }
    // ANCHOR_END: live

    // ANCHOR: anchor-of
    /// The node whose grammar slot `n` is *effectively* filling: climb
    /// through dead ancestors to the first node whose own parent is live
    /// (or the root). See the Perses page for the full story.
    fn anchor_of(
        &self,
        n: NodeId,
        config: &Configuration<Token>,
    ) -> NodeId {
        let mut anchor = n;
        while let Some(&p) = self.node2parent.get(&anchor) {
            if p == self.root || self.live(p, config) {
                break;
            }
            anchor = p;
        }
        anchor
    }
    // ANCHOR_END: anchor-of

    // ANCHOR: can-replace
    fn can_replace(
        &self,
        n: NodeId,
        d: NodeId,
        config: &Configuration<Token>,
    ) -> bool {
        if n == d {
            return false;
        }
        let anchor = self.anchor_of(n, config);
        let d_kind = self.id2node[&d].kind;
        match self.node2parent.get(&anchor) {
            Some(p) if self.id2node[p].kind == Kind::List => {
                is_stmt(d_kind)
            }
            Some(_) => d_kind == self.id2node[&anchor].kind,
            None => false, // the root fills no slot
        }
    }
    // ANCHOR_END: can-replace

    fn subtree_sizes(
        &self,
        config: &Configuration<Token>,
    ) -> HashMap<NodeId, usize> {
        self.id2node
            .keys()
            .map(|&id| {
                (id, self.leaves_under(id, config).len())
            })
            .collect()
    }

    fn live_internal_largest_first(
        &self,
        config: &Configuration<Token>,
        node2size: &HashMap<NodeId, usize>,
    ) -> Vec<NodeId> {
        let mut nodes: Vec<NodeId> = self
            .id2node
            .keys()
            .copied()
            .filter(|&id| {
                !self.id2node[&id].children.is_empty()
                    && self.live(id, config)
            })
            .collect();
        nodes.sort_by(|&a, &b| {
            node2size[&b]
                .cmp(&node2size[&a])
                .then(a.cmp(&b))
        });
        nodes
    }

    /// The still-present elements (children) of a `List` node.
    fn elems_of(
        &self,
        list: NodeId,
        config: &Configuration<Token>,
    ) -> Configuration<NodeId> {
        // `pick_active` only ever selects `List` nodes
        assert!(self.id2node[&list].kind == Kind::List);
        self.id2node[&list]
            .children
            .iter()
            .copied()
            .filter(|&c| self.present(c, config))
            .collect()
    }
}

// ANCHOR: render
/// Render a configuration by concatenating the surviving tokens in source
/// order -- for a parse tree, that *is* the program.
fn render(tree: &Tree, present: &Configuration<Token>) -> String {
    let mut units: Vec<Token> =
        present.iter().copied().collect();
    units.sort_unstable();
    units
        .iter()
        .map(|&u| {
            tree.id2node[&tree.token2leaf[&u]].label
        })
        .collect::<Vec<_>>()
        .join(" ")
}
// ANCHOR_END: render

// ANCHOR: hdd
struct Hdd<'t, F, P> {
    tree: &'t Tree,
    new_minimizer: F,
    level: usize,
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
            move |drop| -> Delta<Token> {
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

// ANCHOR: perses
struct Perses<'t, F, P> {
    tree: &'t Tree,
    new_minimizer: F,
    // the active `List` node
    active: Option<NodeId>,
    // the active node's minimizer, rebuilt when `active` changes
    minimizer: Option<P>,
    // the active node's present elements, in a field so the
    // returned iterator can borrow them (as HDD does per level)
    active_elems: Configuration<NodeId>,
    done: HashSet<NodeId>, // `List` nodes exhausted since a reduction
}

impl<'t, F, P> Perses<'t, F, P>
where
    F: Fn() -> P,
    P: Policy<NodeId>,
{
    fn new(tree: &'t Tree, new_minimizer: F) -> Self {
        Perses {
            tree,
            new_minimizer,
            active: None,
            minimizer: None,
            active_elems: Configuration::new(),
            done: HashSet::new(),
        }
    }

    /// Pick the *active* `List`: the first one in `nodes` (sorted
    /// largest-first) not in `done`. Sticky: keep the current list
    /// until its minimizer declares it minimal or a collapse kills it
    /// (see the Perses page).
    fn pick_active(&mut self, nodes: &[NodeId]) {
        let tree = self.tree;
        if let Some(a) = self.active {
            if !self.done.contains(&a)
                && nodes.contains(&a)
            {
                return; // still minimizing the current list
            }
        }
        let active = nodes.iter().copied().find(|&id| {
            tree.id2node[&id].kind == Kind::List
                && !self.done.contains(&id)
        });
        if active != self.active {
            self.active = active;
            self.minimizer = None;
        }
    }

    /// Replacement candidates, biggest payoff first.
    fn replacements(
        &self,
        nodes: &[NodeId],
        node2size: &HashMap<NodeId, usize>,
        config: &Configuration<Token>,
    ) -> Vec<Delta<Token>> {
        let tree = self.tree;
        let mut reps: Vec<Delta<Token>> = Vec::new();
        for &n in nodes {
            let n_leaves = tree.leaves_under(n, config);
            let mut ds: Vec<NodeId> = tree
                .descendants(n, config)
                .into_iter()
                .filter(|&d| {
                    tree.live(d, config)
                        && tree.can_replace(n, d, config)
                })
                .collect();
            ds.sort_by(|&a, &b| {
                node2size[&a]
                    .cmp(&node2size[&b])
                    .then(a.cmp(&b))
            });
            for d in ds {
                let delta: Delta<Token> = n_leaves
                    .difference(
                        &tree.leaves_under(d, config),
                    )
                    .copied()
                    .collect();
                if !delta.is_empty() {
                    reps.push(delta);
                }
            }
        }
        reps
    }

    /// Deletion candidates from the active `List`'s persisted minimizer.
    fn deletions(
        &mut self,
        config: &Configuration<Token>,
    ) -> impl Iterator<Item = Delta<Token>> {
        let tree = self.tree;
        self.active_elems = match self.active {
            Some(a) => tree.elems_of(a, config),
            None => Configuration::new(),
        };
        if self.minimizer.is_none() {
            self.minimizer = Some((self.new_minimizer)());
        }
        let elems = &self.active_elems;
        let minimizer = self.minimizer.as_mut().unwrap();
        minimizer.propose(elems).map(
            move |drop| -> Delta<Token> {
                drop.iter()
                    .flat_map(|&id| {
                        tree.leaves_under(id, config)
                    })
                    .collect()
            },
        )
    }
}

impl<'t, F, P> Policy<Token> for Perses<'t, F, P>
where
    F: Fn() -> P,
    P: Policy<NodeId>,
{
    fn propose(
        &mut self,
        config: &Configuration<Token>,
    ) -> impl Iterator<Item = Delta<Token>> {
        // biggest payoff first: order the live nodes by surviving size
        let node2size = self.tree.subtree_sizes(config);
        let nodes = self
            .tree
            .live_internal_largest_first(config, &node2size);
        // one List at a time gets a persisted deletion minimizer
        self.pick_active(&nodes);
        // replacements first, then the active List's deletions
        let reps =
            self.replacements(&nodes, &node2size, config);
        reps.into_iter().chain(self.deletions(config))
    }

    fn on_reduced(
        &mut self,
        reduced: Option<&Configuration<Token>>,
    ) -> bool {
        let tree = self.tree;
        match self.active {
            Some(a) => {
                // Forward the outcome to the active minimizer in
                // its own space: the List's still-present elements.
                let elems =
                    reduced.map(|c| tree.elems_of(a, c));
                let inner =
                    self.minimizer.as_mut().unwrap();
                let keep = inner.on_reduced(elems.as_ref());
                if reduced.is_some() {
                    self.done.clear(); // a reduction re-opens all
                    true
                } else if keep {
                    true // the inner policy isn't minimal yet
                } else {
                    // nothing more to delete here: mark the list
                    // done; an all-failing pass then ends the run.
                    self.done.insert(a);
                    true
                }
            }
            None => reduced.is_some(), // replacements-only pass
        }
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
    p: HashMap<NodeId, f64>,
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
/// A static prior from the tree's shape.
fn priors(
    tree: &Tree,
    sigma: f64,
) -> HashMap<NodeId, f64> {
    tree.id2node
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
/// The model's belief that `n`'s subtree contributes no surviving token.
fn q(
    tree: &Tree,
    p: &HashMap<NodeId, f64>,
    config: &Configuration<Token>,
    n: NodeId,
) -> f64 {
    let node = &tree.id2node[&n];
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
fn pass_prob(
    tree: &Tree,
    p: &HashMap<NodeId, f64>,
    config: &Configuration<Token>,
    d: NodeId,
) -> f64 {
    let mut result = q(tree, p, config, d);
    let mut node = d;
    while let Some(&a) = tree.node2parent.get(&node) {
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
/// The expected number of tokens a deletion at `d` removes.
fn expected_gain(
    tree: &Tree,
    p: &HashMap<NodeId, f64>,
    config: &Configuration<Token>,
    d: NodeId,
) -> f64 {
    tree.leaves_under(d, config).len() as f64
        * pass_prob(tree, p, config, d)
}
// ANCHOR_END: expected-gain

// ANCHOR: choose
/// Below this expected gain, T-PDD gives up (the paper's threshold).
const MIN_GAIN: f64 = 1.0;

/// The live node with the highest expected gain, excluding the root and
/// any node already certain to survive (`p = 1`).
fn best_candidate(
    tree: &Tree,
    p: &HashMap<NodeId, f64>,
    config: &Configuration<Token>,
) -> Option<NodeId> {
    let mut cands: Vec<(NodeId, f64)> = tree
        .id2node
        .keys()
        .copied()
        .filter(|&id| {
            id != tree.root
                && p[&id] < 1.0
                && tree.live(id, config)
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
/// A deletion at `d` just failed: raise its belief (the same shape of
/// posterior ProbDD uses).
fn update(
    tree: &Tree,
    p: &mut HashMap<NodeId, f64>,
    config: &Configuration<Token>,
    d: NodeId,
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
impl Policy<Token> for TPdd<'_> {
    fn propose(
        &mut self,
        config: &Configuration<Token>,
    ) -> impl Iterator<Item = Delta<Token>> {
        let tree = self.tree;
        let p = &mut self.p;

        // pulling the next delta means the previous one failed
        let mut last: Option<NodeId> = None;
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
    // The scattered bug: three cooperating calls, far apart.
    let tree = std::rc::Rc::new(example_tree());
    // the starting configuration: every token
    let all: Configuration<Token> =
        (0..tree.token2leaf.len() as Token).collect();

    // The three scattered essentials, found by label.
    let needed: Vec<Token> = ["setup", "corrupt", "crash"]
        .iter()
        .map(|want| {
            tree.id2node
                .iter()
                .find(|(_, n)| n.label == *want)
                .map(|(id, _)| tree.leaf2token[id])
                .unwrap()
        })
        .collect();

    // ANCHOR: make-oracle
    // Interesting iff *all three* scattered calls survive and the
    // program still parses.
    let make_oracle = || {
        let calls =
            std::rc::Rc::new(std::cell::Cell::new(0u32));
        let counter = calls.clone();
        let otree = tree.clone();
        let needed = needed.clone();
        let oracle = move |c: &Configuration<Token>| {
            counter.set(counter.get() + 1);
            let src = render(&otree, c);
            let ok = needed.iter().all(|t| c.contains(t))
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
        "HDD           => {:?}  in {} calls\n",
        render(&tree, &hdd),
        hdd_calls.get()
    );

    let (hddp_oracle, hddp_calls) = make_oracle();
    let hddp = reduce(
        all.clone(),
        &hddp_oracle,
        Hdd::new(&*tree, 0, || ProbDD {
            unit2prob: HashMap::new(),
            p0: 0.1,
        }),
    );
    println!(
        "HDD+ProbDD    => {:?}  in {} calls\n",
        render(&tree, &hddp),
        hddp_calls.get()
    );

    let (perses_oracle, perses_calls) = make_oracle();
    let perses = reduce(
        all.clone(),
        &perses_oracle,
        Perses::new(&*tree, || DDMin),
    );
    println!(
        "Perses        => {:?}  in {} calls\n",
        render(&tree, &perses),
        perses_calls.get()
    );

    let (pp_oracle, pp_calls) = make_oracle();
    let pp = reduce(
        all.clone(),
        &pp_oracle,
        Perses::new(&*tree, || ProbDD {
            unit2prob: HashMap::new(),
            p0: 0.1,
        }),
    );
    println!(
        "Perses+ProbDD => {:?}  in {} calls\n",
        render(&tree, &pp),
        pp_calls.get()
    );

    let (tpdd_oracle, tpdd_calls) = make_oracle();
    let tpdd = reduce(
        all.clone(),
        &tpdd_oracle,
        TPdd::new(&*tree, 0.5),
    );
    println!(
        "T-PDD         => {:?}  in {} calls\n",
        render(&tree, &tpdd),
        tpdd_calls.get()
    );

    let kept = "int main ( ) { setup ( ) ; if ( c1 ) { corrupt ( ) ; if ( c2 ) { crash ( ) ; } } }";
    assert_eq!(render(&tree, &hdd), kept);
    assert_eq!(render(&tree, &hddp), kept);
    assert_eq!(render(&tree, &tpdd), kept);
    assert_eq!(
        render(&tree, &perses),
        "int main ( ) { setup ( ) ; { corrupt ( ) ; crash ( ) ; } }"
    );
    assert_eq!(render(&tree, &pp), render(&tree, &perses));
    assert_eq!(hdd_calls.get(), 12);
    assert_eq!(hddp_calls.get(), 13);
    assert_eq!(perses_calls.get(), 64);
    assert_eq!(pp_calls.get(), 63);
    assert_eq!(tpdd_calls.get(), 8);
}
// ANCHOR_END: main


/// A tiny builder so the nested example reads top-down instead of as a giant map.
struct Builder {
    id2node: HashMap<NodeId, Node>,
    next: u32,
}
impl Builder {
    fn new() -> Builder {
        Builder {
            id2node: HashMap::new(),
            next: 0,
        }
    }
    fn add(
        &mut self,
        kind: Kind,
        label: &'static str,
        children: Vec<NodeId>,
    ) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        self.id2node.insert(
            id,
            Node {
                kind,
                label,
                children,
            },
        );
        id
    }
    fn tok(&mut self, s: &'static str) -> NodeId {
        self.add(Kind::Token, s, vec![])
    }
    fn call(&mut self, name: &'static str) -> NodeId {
        let n = self.tok(name);
        let lp = self.tok("(");
        let rp = self.tok(")");
        let sc = self.tok(";");
        self.add(Kind::Call, "", vec![n, lp, rp, sc])
    }
    fn list(&mut self, elems: Vec<NodeId>) -> NodeId {
        self.add(Kind::List, "", elems)
    }
    fn block(&mut self, list: NodeId) -> NodeId {
        let lb = self.tok("{");
        let rb = self.tok("}");
        self.add(Kind::Block, "", vec![lb, list, rb])
    }
    fn if_stmt(
        &mut self,
        cond_name: &'static str,
        body: NodeId,
    ) -> NodeId {
        let kw = self.tok("if");
        let lp = self.tok("(");
        let c = self.tok(cond_name);
        let cond = self.add(Kind::Expr, "", vec![c]);
        let rp = self.tok(")");
        self.add(
            Kind::IfStmt,
            "",
            vec![kw, lp, cond, rp, body],
        )
    }
    fn func(&mut self, body: NodeId) -> NodeId {
        let t = self.tok("int");
        let m = self.tok("main");
        let lp = self.tok("(");
        let rp = self.tok(")");
        self.add(Kind::Func, "", vec![t, m, lp, rp, body])
    }
}

fn example_tree() -> Tree {
    let mut b = Builder::new();
    // innermost: { crash(); noise(); }
    let crash = b.call("crash");
    let n0 = b.call("noise");
    let l2 = b.list(vec![crash, n0]);
    let blk2 = b.block(l2);
    let if2 = b.if_stmt("c2", blk2);
    // { corrupt(); if (c2) {...} noise(); }
    let corrupt = b.call("corrupt");
    let n1 = b.call("noise");
    let l1 = b.list(vec![corrupt, if2, n1]);
    let blk1 = b.block(l1);
    let if1 = b.if_stmt("c1", blk1);
    // int main() { setup(); if (c1) {...} noise(); }
    let setup = b.call("setup");
    let n2 = b.call("noise");
    let l0 = b.list(vec![setup, if1, n2]);
    let body = b.block(l0);
    let root = b.func(body);
    Tree::new(root, b.id2node)
}
// ANCHOR_END: all

