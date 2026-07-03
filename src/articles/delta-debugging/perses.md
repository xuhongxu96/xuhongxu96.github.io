# Perses

[HDD] reduces a tree by deleting whole subtrees,
which can be an entire function.
That clears noise,
but deleting a subtree can't free a bug from what's nested around it.
Take a bug buried in nested `if`s:

```c
int main() { if (c1) { if (c2) { if (c3) { crash(); } } } }
```

To delete an `if` is to delete `crash()` with it;
whole-subtree deletion is all HDD has. So it's **stuck**.

[Perses] adds one move: **node replacement**.
`main`'s body is a *block*,
and so is the innermost block wrapping `crash()`,
so Perses deletes everything in between and promotes the inner block into
its place: `{ if (c1) { ... { crash(); } ... } }` becomes `{ crash(); }`.
It's still deletion, just of the surrounding wrapper.

## A Parse Tree, by Hand

Here is the toy grammar---just enough for nested `if`s and statement lists:

```text
func    ::= "int" "main" "(" ")" block
block   ::= "{" stmt* "}"
stmt    ::= if_stmt | block | call
if_stmt ::= "if" "(" expr ")" block
call    ::= ident "(" ")" ";"
expr    ::= ident
```

We model the input as a **parse tree**: every token is a leaf,
every internal node carries a **kind**,
and concatenating the surviving tokens is the program.

The [two-space discipline from HDD](./hdd.md) carries over unchanged: the
atomic units are the *tokens*, numbered `0..n` in source order, and the
configuration is a set of unit indices. Internal nodes are `NodeId`s---names
for regions of the program---and never sit in the configuration. Rendering
is now trivial: sort the surviving units ascending and print their labels;
for a parse tree, that *is* the program.

```rust,ignore
{{#include perses.rs:cst}}
```

> [!NOTE]
> The root of HDD's trouble is its **AST**: the `if (` `)` wrapper isn't
> separate nodes at all---there is nothing there to delete.
>
> A **parse tree** or **concrete syntax tree (CST)** makes each token a node,
> which is what node replacement carves away.

## What HDD May Delete

The freedom of a parse tree cuts both ways: now that mandatory tokens are
leaves too, a careless reducer could drop the `if` in `if (cond) { }` and
leave a dangling `(` `)`, or drop a `{` but keep its `}`.

To keep the baseline faithful to the original HDD---which uses an AST and thus
only ever drops what the grammar marks optional---we mark every node
unremovable except a `List` element
(because a Kleene `stmt*` may legally hold fewer statements):

```rust,ignore
{{#include perses.rs:deletable}}
```

HDD consults it where it gathers each level's deletion candidates---the level-`L`
ancestors of the surviving tokens---keeping only the removable ones:

```rust,ignore
{{#include perses.rs:alive}}
```

Now, the `if (cond) { }` wrapper is not a removable subtree,
so HDD has no deletion that drops it and keeps the body.

> [!NOTE]
> We will try to allow HDD to delete any node, not just `List` elements.
> Please see the last section, ["Delete Anything?"](#delete-anything), for that experiment.

## Tracking Node Existence

Before we can build the replacement move, we need an ability the
configuration doesn't give us directly. The configuration records only which
*tokens* survive; it never says whether an *internal* node still exists. Yet
Perses constantly asks exactly that---replacement only makes sense between
nodes that still exist. So the first ingredient is recovering internal-node
existence from the token set.

To see why this is subtle, watch what the promotion from the intro does to
the tree. Take this program:

```c
int main() {
   if (c) {
      crash();
   }
}
```

whose parse tree is:

```text
func
 ├─ "int"
 ├─ "main"
 ├─ "("
 ├─ ")"
 └─ block                   <- n: the node we replace
    ├─ "{"
    ├─ stmt*
    │  └─ if_stmt
    │     ├─ "if"
    │     ├─ "("
    │     ├─ expr
    │     │  └─ "c"
    │     ├─ ")"
    │     └─ block          <- d: kept, promoted into n's place
    │        ├─ "{"
    │        ├─ stmt*
    │        │  └─ call
    │        │     ├─ "crash"
    │        │     ├─ "("
    │        │     ├─ ")"
    │        │     └─ ";"
    │        └─ "}"
    └─ "}"
```

Replacing the outer `block` (`n`) with the inner one (`d`) promotes `crash();` into
`main`'s body:

```c
int main() {
   crash();
}
```

so the parse tree *should* become:

```text
func
 ├─ "int"
 ├─ "main"
 ├─ "("
 ├─ ")"
 └─ block
    ├─ "{"
    ├─ stmt*
    │  └─ call
    │     ├─ "crash"
    │     ├─ "("
    │     ├─ ")"
    │     └─ ";"
    └─ "}"
```

But the tree in our implementation is fixed---we only drop units from the
configuration. So structurally it is unchanged, with `X` marking a dropped
token and `?` an internal node that should be gone yet still sits there:

```text
func
 ├─ "int"
 ├─ "main"
 ├─ "("
 ├─ ")"
 └─ block             <- ?
    ├─ "{"            <- X
    ├─ stmt*          <- ?
    │  └─ if_stmt     <- ?
    │     ├─ "if"     <- X
    │     ├─ "("      <- X
    │     ├─ expr     <- ?
    │     │  └─ "c"   <- X
    │     ├─ ")"      <- X
    │     └─ block
    │        ├─ "{"
    │        ├─ stmt*
    │        │  └─ call
    │        │     ├─ "crash"
    │        │     ├─ "("
    │        │     ├─ ")"
    │        │     └─ ";"
    │        └─ "}"
    └─ "}"            <- X
```

A token's presence is a single config lookup; an internal node's is not. We
have to *recover* it---decide that the `?` nodes are gone even though the tree
still holds them. Three rules, one per kind, do it:

- a **token** (leaf) exists iff the configuration still holds its unit;
- a **regular** node (`Block`, `IfStmt`, `Call`, …) exists iff all its mandatory
  parts do---every non-`List` child (an `if` stops existing the moment any of
  `if ( … )` or its block is gone);
- a **list** (`stmt*`) has no tokens of its own and may legally be empty, so it
  cannot be judged by its own contents; it exists iff the block bracketing it
  does---its parent.

A node that still exists is **live**:

```rust,ignore
{{#include perses.rs:live}}
```

## Node Replacement

With liveness in hand we can build the move itself. It has three parts: what
a replacement removes, which slot the replaced node occupies, and which
descendants fit that slot.

### The Move and Its Delta

Perses replaces a node `n` with one of its descendants `d`: `d` is kept,
everything else under `n` goes. Since the configuration is a set of units,
the delta is just the tokens under `n` minus the tokens under `d`---the
surrounding wrapper:

```rust,ignore
let delta = leaves_under(n) - leaves_under(d);
```

In the promotion above, `n` is `main`'s body and `d` the inner block: the
delta is `{`, `if`, `(`, `c`, `)`, `}`---six wrapper tokens---and one oracle
test later the whole nest is gone.

### Which Slot Is `n` Filling?

Replacement must keep the program grammatical, so before asking whether `d`
fits, we must know which grammar slot it would be filling. The obvious
answer---the slot of `n`'s parent's child, i.e. `n`'s own position---is wrong
as soon as replacements start stacking, because **`n`'s own parent may
already be dead**.

Watch it happen in the demo run. After the promotion above, the live inner
block hangs below a chain of `?` nodes:

```text
func                          (root, live)
 ├─ "int" "main" "(" ")"
 └─ block          <- dead    ┐
    └─ stmt*       <- dead    │ the dead chain
       └─ if_stmt  <- dead    │ anchor_of climbs
          └─ ...   <- dead    ┘
             └─ block         <- live: the promoted { crash(); }
```

If Perses later wants to replace that promoted block, judging it by its
literal parent (a dead `if_stmt`) would conclude "a block in an `if_stmt`
slot"---but that `if_stmt` no longer exists. The block was promoted *into
`main`'s body slot*, and that is the slot any further replacement must keep
filling. `anchor_of` recovers this: climb from `n` up the dead chain to the
first node whose own parent is live (or the root). That ancestor's slot is
the one `n` effectively occupies:

```rust,ignore
{{#include perses.rs:anchor-of}}
```

Tracing it on the picture: start at the promoted block; its parent `if_stmt`
is dead, keep climbing; `stmt*`, dead; the outer block's parent is `func`,
the live root---stop. The anchor is the outer block, whose slot (`func`'s
mandatory body) demands a `Block`.

### Is `d` Compatible?

Now the compatibility test is two rules over the anchor's slot, read
straight from the grammar:

- if the anchor is a `List` element, the slot is `stmt`---any statement kind
  (`IfStmt`, `Block`, `Call`) fits;
- if the anchor is a fixed child (like `func`'s body), the slot admits
  exactly one kind---`d` must match it (a `Block` only a `Block`).

```rust,ignore
{{#include perses.rs:can-replace}}
```

## The Perses Policy

Like HDD, Perses is still a [`Policy`](./ddmin.md): the same `reduce` loop
drives it, and all its strategy lives behind `propose`/`on_reduced`. Its
state is the tree, one *active* `List` with a persisted deletion minimizer
(the same trick HDD uses per level), and a set of retired `List`s:

```rust,ignore
{{#include perses.rs:perses-struct}}
```

We build `propose` out of four small pieces, each answering one question.

### Largest First

*Which node should we spend the next test on?* HDD answered "whatever the
current level holds"; Perses answers "**whatever pays the most**". A node's
payoff is the number of surviving tokens under it, so each pass recomputes
subtree sizes and orders the live internal nodes largest first:

```rust,ignore
{{#include perses.rs:sizes}}
```

```rust,ignore
{{#include perses.rs:live-nodes}}
```

On the demo program's first pass the ordering starts:

| live node                  | surviving tokens |
|----------------------------|------------------|
| `func` (root)              | 48               |
| `block` (`main`'s body)    | 44               |
| `stmt*` (outermost list)   | 42               |
| `if_stmt` (`if (c1) ...`)  | 34               |
| `block` (`if (c1)`'s body) | 30               |
| ⋮                          | ⋮                |

### Generating Replacements

*What is the best replacement to try at each node?* For a node `n`, the
smallest compatible descendant `d` removes the most---so candidates are
emitted per node largest-`n`-first, and within a node smallest-`d`-first:
the biggest jump that still parses comes out of the iterator before the
cautious ones.

```rust,ignore
{{#include perses.rs:perses-replace}}
```

Trace it on `main`'s body (size 44). Its compatible live descendants are the
three nested blocks, tried smallest first: the innermost block (size 10)
first---delta = 34 wrapper tokens. That very first candidate is the one the
oracle accepts on the demo, collapsing all three `if`s in one test.

### One Active List at a Time

*Where does deletion happen?* Deletion needs a stateful minimizer driven
across passes (that is how DDMin escalates granularity and how ProbDD
learns), and state needs a home. Perses keeps **one** persisted minimizer,
attached to the *active* `List`: the largest live list not yet retired.
Because the choice is recomputed every pass, a collapse that kills the
active list never strands the driver---the next pass simply picks the
largest survivor:

```rust,ignore
{{#include perses.rs:perses-active}}
```

The active list's deletions are generated exactly as HDD generates a
level's: hand the minimizer the list's present elements (in node space),
stream its choices lazily, and map each through `leaves_under` into a
unit-delta:

```rust,ignore
{{#include perses.rs:perses-delete}}
```

Assembled, `propose` reads as the four questions in order---order the nodes,
pick the active list, offer replacements, then the active list's deletions:

```rust,ignore
{{#include perses.rs:perses-propose}}
```

> [!WARNING]
> Real Perses bounds the descendant search and can splice a nested list into its
> parent. We keep the descendant search simple and skip the splice---the collapse
> here is pure statement-for-statement replacement.

### Retiring Lists

*When is a list finished---and when is the whole run?* `on_reduced` closes
the loop. Each pass's outcome is forwarded to the active minimizer in its own
space; when the minimizer declares its list minimal, the list is **retired**
into `done` and the next pass moves to the next-largest one. But any
successful reduction re-opens every retired list---a collapse can expose
deletions anywhere:

```rust,ignore
{{#include perses.rs:perses-on-reduced}}
```

Here is the protocol at work on the demo run, pass by pass:

| pass | active list       | first accepted candidate                    | outcome     | `done` after |
|------|-------------------|---------------------------------------------|-------------|--------------|
| 1    | outermost `stmt*` | replace `main`'s body with innermost block  | reduced     | ∅ (re-opened) |
| 2    | innermost `stmt*` | delete `noise();` (keep only `crash();`)    | reduced     | ∅ (re-opened) |
| 3    | innermost `stmt*` | none---deleting `crash();` is rejected      | no progress | { innermost `stmt*` } |
| 4    | none left         | no candidates                               | stop        |              |

## Run It

Time to compare HDD and Perses head to head. The input is the nested-`if` program
from the top, now with one `noise();` call added at every level for them to prune:

```c
int main() {
   if (c1) {
      if (c2) {
         if (c3) { crash(); noise(); }
         noise();
      }
      noise();
   }
   noise(); noise();
}
```

Both run against the same oracle: interesting iff the program still contains
`crash` *and* still parses. It also counts how many times it's called:

```rust,ignore
{{#include perses.rs:make-oracle}}
```

The reducers are **HDD**,
which deletes removable elements,
and **Perses**,
which also deletes a wrapper to promote what's inside (node replacement).

> [!TIP]
> Press play. HDD strips the `noise;` lines but can't break the `if` nesting;
> Perses replaces `main`'s body with the inner block in one test, then clears the
> leftover noise.

```rust,edition2024
{{#rustdoc_include perses.rs:main}}
```

HDD bottoms out at the nesting it can't remove:

```c
int main() { if (c1) { if (c2) { if (c3) { crash(); } } } }
```

Perses collapses it to the core:

```c
int main() { crash(); }
```

> [!NOTE]
> Perses reaches a strictly smaller program---in fewer calls (3 vs 9)---because it
> can delete the wrapper around the bug, not just the noise beside it.
> <!-- kept in sync with the asserts in perses.rs main -->

## Delete Anything?

HDD got stuck only because we let it delete `List` elements and nothing else.
So let's try letting it delete *any* node, not just `List` elements.
The only change is to remove the `deletable` filter from `alive_level_nodes`:

```rust,ignore
{{#include perses_all_deletable.rs:alive-all}}
```

Now it *can* drop an `if (cond)` header and keep the block inside, collapsing
the nest just as Perses did. However, with no grammar to guide it, most of
these new deletions break the program---for example, a `{` left without its
`}`.

> [!TIP]
> Press play to watch HDD grope: most of its candidates get rejected as
> unparsable before one finally collapses the nest.

```rust,edition2024
{{#rustdoc_include perses_all_deletable.rs:main}}
```

It does reach Perses's result:

```c
int main() { crash(); }
```

> [!NOTE]
> Same program---but **105 oracle calls** versus Perses's 3.
> <!-- kept in sync with the assert in perses_all_deletable.rs main -->

[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[T-PDD]: https://ieeexplore.ieee.org/document/10299940
