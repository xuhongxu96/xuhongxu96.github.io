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

/// An indivisible piece of the input: a char, token, line, etc.
trait AtomicUnit: Copy + Eq + std::hash::Hash + Ord {}
impl<T: Copy + Eq + std::hash::Hash + Ord> AtomicUnit for T {}

/// This chapter's atomic unit: a token of the program, identified by
/// its position in source order (0, 1, 2, ...).
type Token = u32;

/// The units we keep.
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

/// Identifies a node of the parse tree. *Not* an atomic unit: internal
/// nodes never appear in a Configuration. A leaf (token) node corresponds
/// to exactly one atomic unit: its source-order index, `tree.leaf2token[&id]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct NodeId(u32);

/// A parse tree: every token is a leaf, so concatenating the surviving tokens
/// *is* the program.
struct Node {
    label: &'static str, // the source text, for a leaf
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

    // ANCHOR: alive-all
    /// The level-`level` subtrees still holding a token -- the candidates HDD
    /// may delete at this level. Every node but the root is eligible.
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
            .filter(|&node| node != self.root)
            .collect()
    }
    // ANCHOR_END: alive-all

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

}

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

/// HDD walks the tree level by level and lets a fresh list-minimizer drop the
/// level's candidates -- here, any node but the root.
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

    // Interesting iff the program still contains crash() *and* still parses.
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
            if ok { "crashes (keep)" } else { "reject" }
        );
        if ok {
            Verdict::Interesting
        } else {
            Verdict::NotInteresting
        }
    };

    let hdd_all = reduce(
        all,
        &oracle,
        Hdd::new(&*tree, 0, || DDMin),
    );
    println!(
        "HDD (all deletable) => {:?}  in {} calls",
        render(&tree, &hdd_all),
        calls.get()
    );

    assert_eq!(
        render(&tree, &hdd_all),
        "int main ( ) { crash ( ) ; }"
    );
    assert_eq!(calls.get(), 105);
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
        label: &'static str,
        children: Vec<NodeId>,
    ) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        self.id2node.insert(id, Node { label, children });
        id
    }
    fn tok(&mut self, s: &'static str) -> NodeId {
        self.add(s, vec![])
    }
    /// `name ( ) ;`  -- an expression statement.
    fn call(&mut self, name: &'static str) -> NodeId {
        let n = self.tok(name);
        let lp = self.tok("(");
        let rp = self.tok(")");
        let sc = self.tok(";");
        self.add("", vec![n, lp, rp, sc])
    }
    fn list(&mut self, elems: Vec<NodeId>) -> NodeId {
        self.add("", elems)
    }
    /// `{ stmts }`
    fn block(&mut self, list: NodeId) -> NodeId {
        let lb = self.tok("{");
        let rb = self.tok("}");
        self.add("", vec![lb, list, rb])
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
        let cond = self.add("", vec![c]);
        let rp = self.tok(")");
        self.add("", vec![kw, lp, cond, rp, body])
    }
    /// `int main ( ) body`
    fn func(&mut self, body: NodeId) -> NodeId {
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
    Tree::new(root, b.id2node)
}
