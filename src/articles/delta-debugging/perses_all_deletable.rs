// Perses, "Delete Anything?" variant: same framework as perses.rs, but HDD's
// `List`-only restriction is lifted -- every node but the root is a deletion
// candidate. A blind deletion can now break the parse (drop a `{`, keep its `}`),
// so the oracle also checks the program still parses. Compiles and runs on its
// own:
//
//     rustc --edition 2024 perses_all_deletable.rs && ./perses_all_deletable

use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::successors;

/// An indivisible piece of the input: here, a node of the parse tree.
type AtomicUnit = u32;

/// The units we keep. Reduction shrinks this set.
type Configuration = HashSet<AtomicUnit>;

#[derive(PartialEq)]
enum Verdict {
    Interesting,    // still triggers the bug
    NotInteresting, // does not trigger the bug or is invalid
}

type Oracle = dyn Fn(&Configuration) -> Verdict;

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

/// A parse tree: every token is a leaf, so concatenating the surviving leaves
/// *is* the program.
struct Node {
    label: &'static str, // the source text, for a leaf
    children: Vec<AtomicUnit>,
}

struct Tree {
    nodes: HashMap<AtomicUnit, Node>,
    root: AtomicUnit,
    depth: HashMap<AtomicUnit, usize>,
    parent: HashMap<AtomicUnit, AtomicUnit>,
    max_depth: usize,
}

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

    // ANCHOR: alive-all
    /// The level-`level` subtrees still holding a leaf -- the candidates HDD may
    /// delete at this level. Every node but the root is eligible (the grammar
    /// restriction to `List` elements is lifted).
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
            .filter(|&node| node != self.root)
            .collect()
    }
    // ANCHOR_END: alive-all

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
}

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

/// HDD walks the tree level by level and lets a fresh list-minimizer drop the
/// level's candidates -- here, any node but the root.
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

    // Interesting iff the program still contains crash() *and* still parses.
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
            if ok { "crashes (keep)" } else { "reject" }
        );
        if ok {
            Verdict::Interesting
        } else {
            Verdict::NotInteresting
        }
    };

    let hdd_all =
        reduce(all, &oracle, Hdd::new(&*tree, 0, || DDMin));
    println!(
        "HDD (all deletable) => {:?}  in {} calls",
        render(&tree, &hdd_all),
        calls.get()
    );

    // Same result as Perses -- but it took far more oracle calls to get there.
    assert_eq!(
        render(&tree, &hdd_all),
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
        label: &'static str,
        children: Vec<AtomicUnit>,
    ) -> AtomicUnit {
        let id = self.next;
        self.next += 1;
        self.nodes.insert(id, Node { label, children });
        id
    }
    fn tok(&mut self, s: &'static str) -> AtomicUnit {
        self.add(s, vec![])
    }
    /// `name ( ) ;`  -- an expression statement.
    fn call(&mut self, name: &'static str) -> AtomicUnit {
        let n = self.tok(name);
        let lp = self.tok("(");
        let rp = self.tok(")");
        let sc = self.tok(";");
        self.add("", vec![n, lp, rp, sc])
    }
    fn list(
        &mut self,
        elems: Vec<AtomicUnit>,
    ) -> AtomicUnit {
        self.add("", elems)
    }
    /// `{ stmts }`
    fn block(&mut self, list: AtomicUnit) -> AtomicUnit {
        let lb = self.tok("{");
        let rb = self.tok("}");
        self.add("", vec![lb, list, rb])
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
        let cond = self.add("", vec![c]);
        let rp = self.tok(")");
        self.add("", vec![kw, lp, cond, rp, body])
    }
    /// `int main ( ) body`
    fn func(&mut self, body: AtomicUnit) -> AtomicUnit {
        let t = self.tok("int");
        let m = self.tok("main");
        let lp = self.tok("(");
        let rp = self.tok(")");
        self.add("", vec![t, m, lp, rp, body])
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
