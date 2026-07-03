// Perses: syntax-guided reduction. Its new move is *node replacement* -- delete a
// node's surrounding wrapper to promote a compatible descendant (a block nested
// inside another block is still a block, so `{ ... { body } ... }` becomes
// `{ body }`). Runs on its own:
//
//     rustc --edition 2024 perses.rs && ./perses

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
// ANCHOR_END: partition

// ANCHOR: ddmin
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
// ANCHOR_END: ddmin

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

    // ANCHOR: leaves-under
    /// Node space -> unit space: the surviving atomic units in the subtree
    /// rooted at `id` (for a leaf, its own token if kept).
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

    /// Does any token under `id` survive?
    fn present(
        &self,
        id: NodeId,
        config: &Configuration<Token>,
    ) -> bool {
        !self.leaves_under(id, config).is_empty()
    }
    // ANCHOR_END: leaves-under

    // ANCHOR: descendants
    /// Every present proper descendant of `id`.
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
    // ANCHOR_END: descendants

    // ANCHOR: alive
    /// The level-`level` subtrees still holding a token -- the candidates HDD
    /// may delete at this level. Each surviving unit's leaf contributes its
    /// level-`L` ancestor; we keep only the `deletable` ones.
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
    /// A node may be deleted only when it is an element of a `List` (Kleene).
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
    /// Does this node still exist in the reduced program? The configuration
    /// tracks only which tokens survive, so an internal node's presence is
    /// recovered here.
    fn live(
        &self,
        id: NodeId,
        config: &Configuration<Token>,
    ) -> bool {
        let node = &self.id2node[&id];
        match node.kind {
            // a token exists iff it is kept
            Kind::Token => {
                config.contains(&self.leaf2token[&id])
            }
            // a list has no tokens of its own -> it exists iff its block does
            Kind::List => self
                .node2parent
                .get(&id)
                .is_some_and(|&p| self.live(p, config)),
            // a regular node exists iff its mandatory (non-list) children do
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
    /// The node whose grammar slot `n` is *effectively* filling. After an
    /// earlier replacement, `n`'s own parent may be dead -- `n` was promoted
    /// into some ancestor's place. Walk up through the dead chain to the
    /// first node whose own parent is live (or the root): that ancestor's
    /// slot is the one `n` occupies now.
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
    /// Can `d` replace `n`? `d`'s kind must fit the slot `n` is effectively
    /// filling: a `List` element's slot accepts any statement; a fixed slot
    /// accepts only its own kind (a `Block` only a `Block`).
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

    // ANCHOR: sizes
    /// How many surviving tokens each node's subtree still holds -- the
    /// payoff of removing or replacing it. Recomputed per pass, since the
    /// payoffs shrink as the program does.
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
    // ANCHOR_END: sizes

    // ANCHOR: live-nodes
    /// The live internal nodes, largest subtree first (ties by id for a
    /// reproducible demo) -- the order in which Perses spends its tests.
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
    // ANCHOR_END: live-nodes

    // ANCHOR: elems-of
    /// The still-present elements (children) of a `List` node -- the
    /// things a deletion minimizer may remove from it.
    fn elems_of(
        &self,
        list: NodeId,
        config: &Configuration<Token>,
    ) -> Configuration<NodeId> {
        self.id2node[&list]
            .children
            .iter()
            .copied()
            .filter(|&c| self.present(c, config))
            .collect()
    }
    // ANCHOR_END: elems-of
}

// ANCHOR: render
/// Render a configuration by concatenating the surviving tokens in source
/// order -- for a parse tree, that *is* the program. Because a unit is its
/// token's source-order index, "in source order" is just ascending units.
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
/// HDD walks the tree level by level and lets a fresh list-minimizer drop the
/// level's *deletable* nodes. Pure deletion -- the baseline.
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

// ANCHOR: perses-struct
/// Perses is a Policy. Largest node first, it proposes **node
/// replacement** (drop everything under `n` except a compatible
/// descendant `d`'s tokens) plus, for `List` nodes, HDD's
/// deletion.
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
// ANCHOR_END: perses-struct

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

    // ANCHOR: perses-active
    /// Pick the *active* `List`: the largest live one not yet retired.
    /// Recomputed every pass, so a collapse that kills a node never
    /// strands the driver on it. Switching away from a node discards
    /// its minimizer.
    fn pick_active(&mut self, nodes: &[NodeId]) {
        let tree = self.tree;
        let active = nodes.iter().copied().find(|&id| {
            tree.id2node[&id].kind == Kind::List
                && !self.done.contains(&id)
        });
        if active != self.active {
            self.active = active;
            self.minimizer = None;
        }
    }
    // ANCHOR_END: perses-active

    // ANCHOR: perses-replace
    /// Replacement candidates, biggest payoff first: for each live node
    /// `n` (largest first), try its compatible live descendants `d`,
    /// smallest first -- the biggest jump that still parses. The delta
    /// is the wrapper: `n`'s tokens minus `d`'s.
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
    // ANCHOR_END: perses-replace

    // ANCHOR: perses-delete
    /// Deletion candidates: drive the active `List`'s persisted
    /// minimizer lazily, exactly as HDD drives one level's. Its present
    /// elements go in a field so the returned iterator can borrow them;
    /// `reduce` stops at the first success, so a stateful inner policy
    /// advances only over confirmed failures.
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
                // dropping a subtree drops the tokens under it
                drop.iter()
                    .flat_map(|&id| {
                        tree.leaves_under(id, config)
                    })
                    .collect()
            },
        )
    }
    // ANCHOR_END: perses-delete
}

impl<'t, F, P> Policy<Token> for Perses<'t, F, P>
where
    F: Fn() -> P,
    P: Policy<NodeId>,
{
    // ANCHOR: perses-propose
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
    // ANCHOR_END: perses-propose

    // ANCHOR: perses-on-reduced
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
                    // a reduction can expose deletions anywhere:
                    // re-open every retired `List`
                    self.done.clear();
                    true
                } else if keep {
                    true // the inner policy isn't minimal yet
                } else {
                    // exhausted: retire it and move to the next.
                    // Once every list is retired, `active` is None
                    // and the next all-failing pass ends the run.
                    self.done.insert(a);
                    true
                }
            }
            None => reduced.is_some(), // replacements-only pass
        }
    }
    // ANCHOR_END: perses-on-reduced
}

// ANCHOR: valid
/// Does the surviving program still parse against the grammar? Once *any* node is
/// deletable, a deletion can land mid-production (drop `{` but keep `}`), so the
/// oracle must also reject programs the grammar no longer accepts.
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
        // func ::= "int" "main" "(" ")" block
        fn func(&mut self) -> bool {
            self.eat("int")
                && self.eat("main")
                && self.eat("(")
                && self.eat(")")
                && self.block()
        }
        // block ::= "{" stmt* "}"
        fn block(&mut self) -> bool {
            if !self.eat("{") {
                return false;
            }
            while self.stmt() {}
            self.eat("}")
        }
        // stmt ::= if_stmt | block | call  (each alternative backtracks on failure)
        fn stmt(&mut self) -> bool {
            let save = self.pos;
            self.if_stmt()
                || (self.reset(save), self.block()).1
                || (self.reset(save), self.call()).1
                || (self.reset(save), false).1
        }
        // if_stmt ::= "if" "(" ident ")" block
        fn if_stmt(&mut self) -> bool {
            self.eat("if")
                && self.eat("(")
                && self.ident()
                && self.eat(")")
                && self.block()
        }
        // call ::= ident "(" ")" ";"
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

// ANCHOR: main
fn main() {
    // nested `if`s, with crash() at the bottom and noise() statements throughout
    let tree = std::rc::Rc::new(example_tree());
    // The configuration is every token of the program: units 0..n in
    // source order. Tree nodes are bookkeeping; they never sit in it.
    let all: Configuration<Token> =
        (0..tree.token2leaf.len() as Token).collect();

    let crash: Token = tree
        .id2node
        .iter()
        .find(|(_, n)| n.label == "crash")
        .map(|(id, _)| tree.leaf2token[id])
        .unwrap();

    // ANCHOR: make-oracle
    // Interesting iff the program still contains crash() *and* still parses. The
    // returned counter tallies how many candidates each reducer tries.
    let make_oracle = || {
        let calls =
            std::rc::Rc::new(std::cell::Cell::new(0u32));
        let counter = calls.clone();
        let otree = tree.clone();
        let oracle = move |c: &Configuration<Token>| {
            counter.set(counter.get() + 1);
            let src = render(&otree, c);
            let ok = c.contains(&crash) && parses(&src);
            println!(
                "  test {src:?}  ->  {}",
                if ok {
                    "crashes (keep)"
                } else {
                    "reject"
                }
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

    // Level 0 is safe here, unlike the HDD chapter's demo: `deletable`
    // filters what HDD may delete, and the root is no List element, so a
    // level-0 pass simply proposes nothing.
    let (hdd_oracle, hdd_calls) = make_oracle();
    let hdd = reduce(
        all.clone(),
        &hdd_oracle,
        Hdd::new(&*tree, 0, || DDMin),
    );
    println!(
        "HDD (Kleene only) => {:?}  in {} calls\n",
        render(&tree, &hdd),
        hdd_calls.get()
    );

    let (perses_oracle, perses_calls) = make_oracle();
    let perses = reduce(
        all.clone(),
        &perses_oracle,
        Perses::new(&*tree, || DDMin),
    );
    println!(
        "Perses            => {:?}  in {} calls\n",
        render(&tree, &perses),
        perses_calls.get()
    );

    // HDD is stuck at the mandatory `if` nesting; Perses collapses it.
    assert_eq!(
        render(&tree, &hdd),
        "int main ( ) { if ( c1 ) { if ( c2 ) { if ( c3 ) { crash ( ) ; } } } }"
    );
    assert_eq!(
        render(&tree, &perses),
        "int main ( ) { crash ( ) ; }"
    );
    assert_eq!(hdd_calls.get(), 9);
    assert_eq!(perses_calls.get(), 3);
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
    /// `name ( ) ;`  -- an expression statement.
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
    /// `{ stmts }`
    fn block(&mut self, list: NodeId) -> NodeId {
        let lb = self.tok("{");
        let rb = self.tok("}");
        self.add(Kind::Block, "", vec![lb, list, rb])
    }
    /// `if ( cond ) body`
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
    /// `int main ( ) body`
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
    let l3 = b.list(vec![crash, n0]);
    let blk3 = b.block(l3);
    let if3 = b.if_stmt("c3", blk3);
    // { if (c3) {...} noise(); }
    let n1 = b.call("noise");
    let l2 = b.list(vec![if3, n1]);
    let blk2 = b.block(l2);
    let if2 = b.if_stmt("c2", blk2);
    // { if (c2) {...} noise(); }
    let n2 = b.call("noise");
    let l1 = b.list(vec![if2, n2]);
    let blk1 = b.block(l1);
    let if1 = b.if_stmt("c1", blk1);
    // int main() { if (c1) {...} noise(); noise(); }
    let n3 = b.call("noise");
    let n4 = b.call("noise");
    let l0 = b.list(vec![if1, n3, n4]);
    let body = b.block(l0);
    let root = b.func(body);
    Tree::new(root, b.id2node)
}
// ANCHOR_END: all
