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

## Two Spaces: Nodes and Units

If the input is now a tree, what should the *configuration*---the set that
reduction shrinks---contain?

Not tree nodes. The atomic units are still exactly what they were in
[DDMin]: the indivisible pieces of the **input**. For a program those are
its tokens, and we identify each token by its position in source order:

```rust,ignore
{{#include hdd.rs:unit}}
```

The tree is a separate, static map *over* those units, so its nodes get
their own id type. An internal node like `fn bar` spans many tokens,
but it is not itself an input---it is a *name for a region* of the
input---so it must never be confused with one of the input's tokens.
Only the leaves touch the input: each leaf
node corresponds to one token,
recorded in the `leaf2token` map (and its inverse, `token2leaf`):

```rust,ignore
{{#include hdd.rs:tree}}
```

In the demo tree used below, the tokens number like this:

```text
program
├─ fn bar                    (the bug)
│  ├─ stmt b1        unit 0
│  ├─ if guard
│  │  ├─ stmt g      unit 1
│  │  └─ crash()     unit 2
│  └─ stmt b2        unit 3
├─ fn f2 { stmt; stmt; }     units 4, 5
├─ fn f3 { stmt; stmt; }     units 6, 7
├─ fn f4 { stmt; stmt; }     units 8, 9
├─ fn f5 { stmt; stmt; }     units 10, 11
└─ fn f6 { stmt; stmt; }     units 12, 13
```

The starting configuration is simply every token: `{0, 1, ..., 13}`. The
tree never shrinks; only the configuration does. That split means HDD keeps
working in two spaces at once---nodes to *decide*, units to *test*---and it
needs one bridge in each direction.

**Node space → unit space.** When HDD decides to try dropping the subtree
`fn f2`, the `reduce` loop can't test a node: a `Delta` is a set of units.
`leaves_under` translates the decision into a testable delta by collecting
the surviving units inside the subtree---dropping `fn f2` means the delta
`{4, 5}`:

```rust,ignore
{{#include hdd.rs:leaves-under}}
```

**Unit space → node space.** Going the other way, HDD must ask which
level-`L` subtrees still *exist*: a node whose tokens have all been deleted
is gone, even though the static tree still has it. Rather than store
liveness separately (state that could drift out of sync), we recover it
from the configuration itself: walk each surviving unit's leaf up to its
ancestor at level `L`. If units `{0,...,5}` survive, level 1 holds
`{fn bar, fn f2}`; delete units 4 and 5 and it holds only `{fn bar}`:

```rust,ignore
{{#include hdd.rs:alive-level}}
```

## A Policy Over Subtrees

HDD's plan is to reuse a plain list-minimizer at every level: hand it the
set of live level-`L` subtrees and let it discover which of them are
removable. And nothing stands in the way, because an *atomic unit is
relative to the reduction problem*. For the inner minimizer's
problem---shrink this level's list of subtrees---the indivisible pieces
are the subtrees themselves, so `NodeId` serves as its atomic unit: the
inner minimizer runs as a `Policy<NodeId>`, and `DDMin`/`ProbDD` satisfy
it unchanged.

HDD itself is a `Policy<Token>` toward the `reduce` loop: whatever the
inner policy decides in node space is expanded through `leaves_under`
into a token-delta before the oracle ever sees it.

## HDD Is a Policy {#hdd-is-a-policy-loop}

**HDD is itself just another `Policy`**---to the `reduce` loop it looks
exactly like DDMin: a stream of unit-deltas. All the hierarchy hides behind
`propose`. Its state is the tree, a factory that builds a fresh inner
minimizer, a cursor for the shallowest level not yet known to be minimal,
and the minimizer currently working that level:

```rust,ignore
{{#include hdd.rs:hdd-struct}}
```

`propose` is the delegation step. For the current level it names the live
subtrees with `alive_level_nodes`, lets the inner policy (`DDMin`,
`ProbDD`, ...) choose which *nodes* to drop, and maps each choice down
through `leaves_under` into the unit-delta the loop can test:

```rust,ignore
{{#include hdd.rs:hdd-propose}}
```

Why stream instead of `collect`ing the inner policy's candidates in one
batch? Pulling the next candidate from `propose` is itself the signal that
the previous one failed. A stateful policy like ProbDD updates its model on
every failure, and because `self.minimizer` is reused across a level's
passes, it carries that learning from one pass to the next. Streaming
*lazily* means the inner policy advances only as the oracle consumes
candidates, so the model learns only from failures that actually happened.

Deciding when to descend is delegated the same way. When a pass ends, HDD
forwards the outcome to the inner policy's own `on_reduced`---translated
into the inner policy's space, this level's live subtrees---and descends
only when the inner policy declares *itself* minimal:

```rust,ignore
{{#include hdd.rs:hdd-on-reduced}}
```

HDD never hard-codes the stop test, so any list-minimizer---stateless or
learning---drives the descent.

To make the delegation concrete, here is the first pass on the demo tree.
The level-1 live subtrees are all six functions; the inner DDMin's first
candidate keeps the half `{fn bar, fn f2, fn f3}`, i.e. proposes dropping
the node-set `{fn f4, fn f5, fn f6}`; `leaves_under` turns that into the
unit-delta `{8, ..., 13}`; the oracle still sees `crash()` (unit 2), so
half the noise disappears in a single test.

```text
level 1 subtrees   {fn bar, f2, f3, f4, f5, f6}
inner DDMin drops  {f4, f5, f6}            (node space)
leaves_under       {8, 9, 10, 11, 12, 13}  (unit space)
oracle             still crashes  =>  reduced
```

> [!NOTE]
> Because HDD only ever removes whole subtrees, every candidate it hands the
> oracle is a syntactically valid tree. The original [DDMin] on a flattened
> token list would spend most of its tests on inputs that don't even parse;
> HDD never wastes a test on a parse error.

> [!NOTE]
> The demos start HDD at **level 1**, not level 0. Level 0 holds only the
> root, and deleting the whole program can never stay interesting, so a
> level-0 pass is guaranteed wasted work. ([Perses](./perses.md) will make
> level 0 harmless in a different way: a grammar-driven filter on what may
> be deleted at all.)

## Run It

The input is the tree above: `fn bar` holds the bug---an `if` whose body
calls `crash()`---and the other five functions are noise.

Keeping the `crash()` token keeps its whole ancestor chain, so the answer
must be `program → fn bar → if → crash()`.

> [!TIP]
> Press play. Watch the first few tests delete whole functions at the top
> level (one test each), then watch HDD descend into `fn bar` and trim it
> down, coarse-to-fine.

```rust,edition2024
{{#rustdoc_include hdd.rs:main}}
```

The top level goes first: `fn f4`, `fn f5`, and `fn f6`, then `fn f3`, then
`fn f2` are dropped---each whole function subtree gone in a single test.
Only then does HDD step inside the surviving `fn bar`, drop its stray
statements, and finally, one level deeper, drop the `if`'s body.
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
> ProbDD reaches the same result---but in **15** calls, *more* than DDMin's
> 11. <!-- kept in sync with the asserts in hdd.rs main -->
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
