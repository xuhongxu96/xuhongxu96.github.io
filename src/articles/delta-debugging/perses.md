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
and concatenating the surviving leaves is the program.

```rust,ignore
{{#include perses.rs:cst}}
```

> [!NOTE]
> The root of HDD's trouble is its **AST**: the `if (` `)` wrapper isn't
> separate nodes at all---there is nothing there to delete.
>
> A **parse tree** or **concrete syntax tree (CST)** makes each token a node,
> which is what node replacement carves away.

The freedom of a parse tree lets HDD delete a *mandatory* token too.
For example, it can drop the `if` in `if (cond) { }`, leaving a dangling `(` and `)`, or drop a `{` but keep the `}`.

To keep the baseline faithful to the original HDD---which uses an AST and thus
only ever drops what the grammar marks optional---we mark every node
unremovable except a `List` element
(because a Kleene `stmt*` may legally hold fewer statements).

```rust,ignore
{{#include perses.rs:deletable}}
```

HDD consults it where it gathers each level's deletion candidates---the level-`L`
ancestors of the surviving leaves---keeping only the removable ones:

```rust,ignore
{{#include perses.rs:alive}}
```

Now, the `if (cond) { }` wrapper is not a removable subtree,
so HDD has no deletion that drops it and keeps the body.

> [!NOTE]
> We will try to allow HDD to delete any node, not just `List` elements.
> Please see the last section, ["Delete Anything?"](#delete-anything), for that experiment.

## Node Replacement

Perses tries to replace a node `n` with a compatible descendant `d`.
If `n` is a `List` element, `d` can be any statement; if `n` is a fixed child, `d` must be the same kind as `n`.

```rust,ignore
{{#include perses.rs:subsume}}
```

> [!NOTE]
> See the next subsection,
> ["Tracking Node Existence"](#tracking-node-existence),
> for what a live node is, why we need to check it, and
> how Perses determines whether a node is live or not (`self.live(n, config)`).

As our *configuration* is a set of leaves,
the *delta* is just the leaves under `n` minus the leaves under `d`---the
surrounding wrapper:

```rust,ignore
let delta = leaves_under(n) - leaves_under(d);
```

### Tracking Node Existence

The configuration records only which *leaf* nodes survive; it never tells us
directly whether an *internal* node still exists. Yet replacement only makes sense
on a node that does, so Perses has to recover that fact.

For example, take this program:

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

But the tree in our implementation is fixed---we only drop leaves from the
configuration. So structurally it is unchanged, with `X` marking a dropped leaf and
`?` an internal node that should be gone yet still sits there:

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

A leaf's presence is a single config lookup; an internal node's is not. We have to
*recover* it---decide that the `?` nodes are gone even though the tree still holds
them. Three rules, one per kind, do it:

- a **token** (leaf) exists iff the configuration still holds it;
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

## Perses Policy

Like HDD, Perses is still a [`Policy`](./ddmin.md).
Each `propose` first takes the live nodes, largest subtree first:

```rust,ignore
{{#include perses.rs:perses-nodes}}
```

For each, the new move---replace it with the smallest compatible descendant, the
biggest jump that still parses:

```rust,ignore
{{#include perses.rs:perses-replace}}
```

and, for a list node, the deletion HDD also has---handed to a fresh inner
minimizer over the present elements, exactly as HDD does per level:

```rust,ignore
{{#include perses.rs:perses-delete}}
```

> [!WARNING]
> Real Perses bounds the descendant search and can splice a nested list into its
> parent. We keep the descendant search simple and skip the splice---the collapse
> here is pure statement-for-statement replacement.

The complete `Perses` policy is shown below.

```rust,ignore
{{#include perses.rs:perses}}
```

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

[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[T-PDD]: https://ieeexplore.ieee.org/document/10299940
