# Hierarchical Delta Debugging

[DDMin] and [ProbDD] both see the input as one flat list of atomic units.
But real failing inputs---programs, HTML, JSON---are **trees**. Flattening a
tree throws away exactly the structure that tells us where to cut: a single
node high in the tree can stand for thousands of atomic units below it.

[HDD] keeps the tree. It walks the syntax tree level by level, from the root
down, and at each level it asks an ordinary **list-minimizer** which of that
level's nodes to drop. Dropping a node drops its whole subtree, so one
test high in the tree can delete a huge, irrelevant region at once---and every
candidate it produces is still a syntactically valid tree.

## A Tree of Units

The atomic units---the things minimization actually removes---are the
**leaves**. A configuration is the set of surviving leaves; internal nodes never
sit in it.

A node is a label plus its children; the whole tree is those nodes
indexed by id, with each node's BFS depth (its **level**) and parent precomputed.

```rust,ignore
{{#include hdd.rs:tree}}
```

HDD needs just two operations on the tree:

1. Given a level-`L` *subtree* (named by
    its root node), `leaves_under` collects the surviving leaves beneath it---this is
    how dropping a subtree becomes a removal of atomic units; and
2. `alive_level_nodes` names the subtrees at a level that still hold a leaf,
    by walking each surviving leaf up to its level-`L` ancestor.

```rust,ignore
{{#include hdd.rs:tree-ops}}
```

## HDD Is a Policy {#hdd-is-a-policy-loop}

**HDD is just another [`Policy`](./ddmin.md)**.
It plugs into the same `reduce` loop as DDMin and ProbDD.

Recall the loop of delta debugging:

```rust,ignore
{{#include hdd.rs:loop}}
```

A [`Policy`](./ddmin.md) streams candidate removals, and reacts to each pass via
`on_reduced` to decide whether the run continues:

```rust,ignore
{{#include hdd.rs:policy}}
```

HDD implements this trait. Its state is the tree, a factory that builds a
list-minimizer (DDMin, ProbDD, etc.), a cursor for the shallowest level not yet
known to be minimal, and the minimizer it is currently using for that level:

```rust,ignore
{{#rustdoc_include hdd.rs:hdd}}
```

For the current level
it names the live subtrees with `alive_level_nodes`
and lets an **inner policy**
(`DDMin`, `ProbDD`, ...) choose which to drop.
Notice that the inner policy chooses among *subtrees*, not raw leaves.
However, the configuration is leaves,
so each chosen subtree is mapped down
through `leaves_under` to the atomic units it removes.

Pulling the next candidate from `propose` is itself the signal that the
previous one failed, so we can't `collect` the inner policy's candidates in one
batch. A stateful policy like ProbDD updates its model (each unit's probability
of being *essential*) on every failure, and because `self.minimizer` is reused
across a level's passes, it carries that learning from one pass to the next.
Since the candidates are streamed *lazily*, the inner policy advances only as
the oracle consumes them, so the model learns only from the failures that
actually happen.

> [!NOTE]
> Because HDD only ever removes whole subtrees, every candidate it hands the
> oracle is a syntactically valid tree. The original [DDMin] on a flattened
> token list would spend most of its tests on inputs that don't even parse;
> HDD never wastes a test on a parse error.

## Run It

The input is a tiny "program" tree.

```text
program {
    fn bar {
        stmt;
        if guard {
            stmt;
            crash();
        }
        stmt;
    }
    fn f2 { stmt; stmt; }
    fn f3 { stmt; stmt; }
    fn f4 { stmt; stmt; }
    fn f5 { stmt; stmt; }
    fn f6 { stmt; stmt; }
}
```

`fn bar` holds the bug---an `if` whose body
calls `crash()`---and the other five functions are noise.

Keeping that node keeps its whole ancestor chain, so the answer must be
`program → fn bar → if → crash()`.

> [!TIP]
> Press play. Watch the first few tests delete whole functions at the top level
> (one test each), then watch HDD descend into `fn bar` and trim it down,
> coarse-to-fine.

```rust,edition2024
{{#rustdoc_include hdd.rs:main}}
```

The top level goes first: `fn f4`, `fn f5`, and `fn f6`, then `fn f3`, then `fn f2` are
dropped---each whole function subtree gone in a single test. Only then does HDD
step inside the surviving `fn bar`, drop its stray statements, and finally, one
level deeper, drop the `if`'s body.
Rendered back into the tree it came from, that is:

```text
program { fn bar { if guard { crash() } } }
```

## Swapping the Minimizer

The inner minimizer is a constructor argument, so swapping [DDMin] for [ProbDD]
is one line---`|| DDMin` becomes `|| ProbDD { probs: HashMap::new(), p0: 0.1 }`,
and nothing else changes. The demo above already runs both and prints each
count.

> [!NOTE]
> ProbDD reaches the same result---but in **15** calls, *more* than DDMin's 11.
>
> That is not a bug. HDD hands the inner policy a fresh, tiny
> list at every level and rebuilds it from scratch each round, so ProbDD's
> probability model---its whole advantage---never has room to learn, and never
> carries information from one level of the tree to the next.
>
> The hierarchy and the statistics never talk. Closing that gap would
> need a policy that reasons across the whole tree at once,
> a different problem than the one explored here.

## On Minimality

DDMin on a flat list guarantees [1-minimality]: no single unit can be removed.
HDD inherits a weaker, tree-shaped cousin. Because it walks top-down and never
revisits a level, it guarantees only that no single *subtree* can be removed
*given the levels above it*---**1-tree-minimality**. A subtree high in the tree
might have become removable only after something below it was cut, and plain HDD
won't go back to find out. Variants like HDD+ and HDD\* iterate to close that
gap; [Perses] attacks it with the grammar.

[1-minimality]: ./ddmin.md
[DDMin]: https://dl.acm.org/doi/10.1109/32.988498
[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[WDD]: https://dl.acm.org/doi/10.1109/ICSE55347.2025.00071
[ProbDD]: https://dl.acm.org/doi/10.1145/3468264.3468625
