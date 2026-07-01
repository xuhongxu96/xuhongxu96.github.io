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

// ANCHOR: partition
/// Split `config` into at most `n` roughly-equal, disjoint subsets.
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
// ANCHOR_END: partition

// ANCHOR: ddmin
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

struct Node {
    kind: Kind,
    label: &'static str, // the source text, for a `Token` leaf
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
    /// The present leaves in the subtree rooted at `id` (for a leaf, itself).
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

    /// Does any leaf under `id` survive?
    fn present(
        &self,
        id: AtomicUnit,
        config: &Configuration,
    ) -> bool {
        !self.leaves_under(id, config).is_empty()
    }

    /// Every present proper descendant of `id`.
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
    /// The level-`level` subtrees still holding a leaf -- the candidates HDD may
    /// delete at this level. Each surviving leaf contributes its level-`L`
    /// ancestor; we keep only the `deletable` ones.
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
    /// A node may be deleted only when it is an element of a `List` (Kleene).
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
    /// Can `d` replace `n`? Its kind must fit the slot `n` is *effectively*
    /// filling: a list element accepts any statement; a fixed child only its
    /// own kind (a `Block` only a `Block`).
    fn can_replace(
        &self,
        n: AtomicUnit,
        d: AtomicUnit,
        config: &Configuration,
    ) -> bool {
        if n == d {
            return false;
        }
        // `n`'s own parent may no longer be *live*.
        // Walk up through that dead chain to the ancestor `n`
        // is actually filling in for: the first
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
    /// Does this node still exist in the reduced program? The configuration tracks
    /// only which leaves survive, so an internal node's presence is recovered here.
    fn live(
        &self,
        id: AtomicUnit,
        config: &Configuration,
    ) -> bool {
        let node = &self.nodes[&id];
        match node.kind {
            // a token exists iff it is kept
            Kind::Token => config.contains(&id),
            // a list has no tokens of its own -> it exists iff its block does
            Kind::List => self
                .parent
                .get(&id)
                .is_some_and(|&p| self.live(p, config)),
            // a regular node exists iff its mandatory (non-list) children do
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
/// Render a configuration by concatenating the surviving leaf tokens in source
/// order -- for a parse tree, that *is* the program.
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
/// HDD walks the tree level by level and lets a fresh list-minimizer drop the
/// level's *deletable* nodes. Pure deletion -- the baseline.
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
/// Perses is a Policy. Largest node first, it proposes HDD's deletion (drop
/// `List` elements, via the same inner minimizer) plus **node replacement**: drop
/// everything under `n` except a compatible descendant `d`'s leaves.
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

        // ANCHOR: perses-nodes
        // the present, *live* internal nodes, largest subtree first
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
            // for each live internal node sorted by size
            // ANCHOR_END: perses-nodes
            // ANCHOR: perses-replace
            let n_leaves = tree.leaves_under(n, config);

            // replace n with a compatible, live descendant -- smallest one first
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
            // ANCHOR_END: perses-replace

            // ANCHOR: perses-delete
            // or, for a list, the deletion HDD also has -- handed to a fresh inner
            // minimizer over the present elements, exactly as HDD does per level
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
        } // end of `for n in nodes`
        cands.into_iter()
        // ANCHOR_END: perses-delete
    }
}
// ANCHOR_END: perses

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

    // ANCHOR: make-oracle
    // Interesting iff the program still contains crash() *and* still parses. The
    // returned counter tallies how many candidates each reducer tries.
    let make_oracle = || {
        let calls =
            std::rc::Rc::new(std::cell::Cell::new(0u32));
        let counter = calls.clone();
        let otree = tree.clone();
        let oracle = move |c: &Configuration| {
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
        Perses {
            tree: &*tree,
            new_minimizer: || DDMin,
        },
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
    /// `name ( ) ;`  -- an expression statement.
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
    /// `{ stmts }`
    fn block(&mut self, list: AtomicUnit) -> AtomicUnit {
        let lb = self.tok("{");
        let rb = self.tok("}");
        self.add(Kind::Block, "", vec![lb, list, rb])
    }
    /// `if ( cond ) body`
    fn if_stmt(
        &mut self,
        cond_name: &'static str,
        body: AtomicUnit,
    ) -> AtomicUnit {
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
    Tree::new(root, b.nodes)
}
// ANCHOR_END: all
