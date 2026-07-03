# T-PDD

[HDD]'s closing note flagged a gap: its inner minimizer---DDMin, or the
probabilistic [ProbDD]---is rebuilt from scratch at *every level*. Whatever
it learned about this level's noise is thrown away the instant HDD steps
down to the next. The hierarchy and the statistics never talk.

[T-PDD] is the policy that lets them talk. It keeps **one
Bayesian belief over every node in the tree, for the whole run**.
A failure updates the belief,
and the search moves on still holding everything else it has learned.

The model is built from the tree's own shape:
a `List` element---a Kleene-star child---is *optional*,
so removing one is plausible;
everything else is *mandatory*, and is not.

T-PDD reuses the exact same parse tree Perses built (`Kind`, `NodeId`,
`Node`, `Tree`, `live`, `leaves_under`) *and the exact same nested-`if`
example*---see the [Perses page](./perses.md) if you haven't read it---and
turns that structural fact into a probability.

> [!CAUTION]
> The [T-PDD] paper and its official artifact don't fully agree: the shipped code
> adds node replacement, and tuning constants the
> paper never mentions. This chapter follows the paper's model, not the
> artifact.

## A Prior From the Tree

Every present node other than the root gets a *conditional* retention
probability:

$$
p_n = P(n\text{ survives} \mid n\text{'s parent survives})
$$

The `Tree`'s own
`deletable` check (see [Perses](./perses.md#what-hdd-may-delete))
turns out to be exactly the boolean the prior needs: a
`List` element gets the hyperparameter $\sigma$; everything else is
mandatory, $p = 1$.

```rust,ignore
{{#include t-pdd.rs:priors}}
```

On the nested-`if` demo tree (with $\sigma = 0.5$; mandatory wrapper
tokens and blocks elided from the drawing, all at prior $1.0$):

```text
func                                  (root -- no prior)
└─ { stmt* }                          1.0
   ├─ if (c1) { stmt* }               0.5   <- List elements
   │  ├─ if (c2) { stmt* }            0.5
   │  │  ├─ if (c3) { stmt* }         0.5
   │  │  │  ├─ crash();               0.5
   │  │  │  └─ noise();               0.5
   │  │  └─ noise();                  0.5
   │  └─ noise();                     0.5
   ├─ noise();                        0.5
   └─ noise();                        0.5
```

## How Likely Is a Deletion To Still Pass?

Removing everything under $d$ should be tried in proportion to how likely
the result still passes. That probability has two sources: $d$'s subtree
may already be empty (an optional descendant vanished on its own), or $d$
may never be reached because an ancestor was removed first. Compute each
separately.

A node's subtree is empty only if every one of its live children's subtrees
is empty too, so define this recursively for every node $n$ under $d$,
computed bottom-up starting from $n = d$:

$$
q(n) =
\begin{cases}
1 - p_n & \text{if } n \text{ has no live children} \\[4pt]
(1 - p_n) + p_n \cdot \displaystyle\prod_{c\ \in\ \text{live children}(n)} q(c) & \text{otherwise}
\end{cases}
$$

```rust,ignore
{{#include t-pdd.rs:q}}
```

Two concrete values from the demo tree's first pass:

- a `noise();` call: its children are four mandatory tokens, each with
  $q = 1 - 1 = 0$, so the product is $0$ and
  $q = (1 - 0.5) + 0.5 \cdot 0 = 0.5$---the only way this subtree
  disappears is the call itself being deleted;
- `if (c1) …`: the mandatory `if` token again zeroes the product, so
  $q = 0.5$ too, whether or not anything below it is optional.

$q(d)$ only looks down from $d$.
If a parent above it was removed first,
$d$ is still gone.

Fold $q(d)$ up through $d$'s ancestors to the
root (the paper's "extended graph" $G_{d\text{-}EX}$) to add that in.
Writing $a_0 = d,\ a_1, \dots, a_k$ for that path:

$$
P_0 = q(d), \qquad P_i = (1 - p_{a_i}) + p_{a_i} \cdot P_{i-1}
$$

$\text{pass}(d) = P_k$ is the result.

```rust,ignore
{{#include t-pdd.rs:pass-prob}}
```

Watch the fold work for the *innermost* `noise();`, whose ancestor path
climbs through all three `if`s (the mandatory lists and blocks in between
have $p = 1$ and leave $P$ unchanged):

$$
0.5
\;\xrightarrow{\;\text{if}(c3)\;}\; 0.75
\;\xrightarrow{\;\text{if}(c2)\;}\; 0.875
\;\xrightarrow{\;\text{if}(c1)\;}\; 0.9375
$$

Every optional ancestor adds another escape route---"maybe the whole `if`
goes instead"---so the deeper a node is wrapped in optional structure, the
*higher* its chance that deleting it would still pass.

$\text{pass}(d)$ alone doesn't measure how much a deletion is worth:
removing one token at 99% matters less than removing a hundred at 90%. Weight
it by token count for the **expected gain**:

$$
\text{gain}(d) = |\text{leaves\_under}(d)| \cdot \text{pass}(d)
$$

```rust,ignore
{{#include t-pdd.rs:expected-gain}}
```

## Picking the Best Candidate

Like [ProbDD]'s `best_prefix`, sort by the score---here, expected gain,
descending. Two kinds of node are excluded outright: the root (it has no
parent to condition on, so no entry in $p$), and any node the model is
already *certain* survives ($p = 1$)---mandatory nodes start certain, and a
failed candidate gets pinned certain, so deleting either is a test whose
answer the model already knows.

```rust,ignore
{{#include t-pdd.rs:choose}}
```

Here is the full first-pass ranking on the demo tree
<!-- numbers verified against the run of t-pdd.rs -->:

| candidate (`List` element) | tokens | $\text{pass}$ | gain |
|----------------------------|--------|---------------|------|
| `if (c2) …`                | 24     | 0.75          | **18** |
| `if (c1) …`                | 34     | 0.5           | 17   |
| `if (c3) …`                | 14     | 0.875         | 12.25 |
| `crash();`                 | 4      | 0.9375        | 3.75 |
| `noise();` (innermost)     | 4      | 0.9375        | 3.75 |
| `noise();` (in `c2`'s block) | 4    | 0.875         | 3.5  |
| `noise();` (in `c1`'s block) | 4    | 0.75          | 3    |
| `noise();` (top level, ×2) | 4      | 0.5           | 2    |

Note who wins: **not** the biggest subtree. `if (c1)` holds 34 tokens but
sits directly in `main`'s body, with no optional ancestor to take the
blame---$\text{pass} = 0.5$. `if (c2)` holds fewer tokens (24) but its
extra escape route ($\text{pass} = 0.75$) more than makes up for it. The
model weighs *how much would go* against *how plausibly it can go*.

> [!NOTE]
> Unlike [ProbDD]'s `best_prefix`, which can combine units from anywhere in
> the input, a T-PDD candidate is always one node's whole subtree---never a
> combination across subtrees.

## Learning From Failure

When a candidate $d$ fails,
at least one token under it was essential after all.
So the belief $p_d$ must be updated (raised) to reflect that:

$$
p_d \leftarrow \frac{p_d}{1 - \text{pass}(d)}
$$

```rust,ignore
{{#include t-pdd.rs:update}}
```

Concretely: the first pick, `if (c2)`, fails (it holds `crash()`). Its
belief becomes $0.5 / (1 - 0.75) = 2$, clamped to $1$---**pinned**. And the
pin *propagates* through every later $\text{pass}$ computation: once
`if (c1)` is pinned too, any node whose only escape route ran through those
`if`s drops to $\text{pass} = $ whatever its remaining optional ancestors
provide. Nothing under a fully-pinned spine is ever retried; the knowledge
from one failure prices every future candidate.

> [!NOTE]
> A success needs no update: the node leaves the configuration entirely, so
> its belief becomes irrelevant and is simply never consulted again.

## T-PDD Policy

The state is the tree plus the persistent belief map.

```rust,ignore
{{#include t-pdd.rs:model}}
```

`propose` is the same "next-pull-means-previous-failed" trick [ProbDD](./probdd.md#probdd-policy) uses,
just over a single node id instead of a prefix of a sorted list: on
re-entry, update the last candidate's belief, then pick and stash the new
best.

```rust,ignore
{{#rustdoc_include t-pdd.rs:propose}}
```

## Run It

The input is the Perses page's nested-`if` program, unchanged:

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

All four reducers share the same oracle---interesting iff the program still
contains `crash` and still parses:

```rust,ignore
{{#include t-pdd.rs:make-oracle}}
```

We compare **HDD**, **HDD+ProbDD**, **Perses**, and **T-PDD**.

> [!TIP]
> Press play and watch T-PDD's two phases. First it *probes the spine*,
> top-ranked candidate first: `if (c2)`, then `if (c1)`, then `if (c3)`,
> then `crash();`---four failures, each pinning one belief. Then it
> harvests: the five `noise();` calls fall in five straight tests, one
> each, never retrying anything it already knows.

```rust,edition2024
{{#rustdoc_include t-pdd.rs:main}}
```

```text
HDD        => 9 calls   (stuck at the nested ifs)
HDD+ProbDD => 14 calls  (stuck at the nested ifs)
Perses     => 3 calls   (collapses the nest)
T-PDD      => 9 calls   (stuck at the nested ifs)
```
<!-- kept in sync with the asserts in t-pdd.rs main -->

> [!NOTE]
> On an input this small, T-PDD's count merely ties plain HDD---but compare
> the *anatomy* of the runs. HDD spends its 9 calls testing batches level by
> level; T-PDD spends 4 calls learning exactly which nodes are essential and
> 5 calls deleting everything else, with zero wasted retests. That
> learning-then-harvesting shape is what scales: the model's knowledge
> survives across the whole run, and a pinned belief re-prices every
> candidate beneath it for free.
>
> HDD+ProbDD is *worse* than plain HDD here (14 calls): each level rebuilds
> the probability model from scratch and re-pays the same learning cost, and
> its batched probes raise whole batches of beliefs on every failure. A
> probabilistic model helps only if it is allowed to remember---which is
> precisely T-PDD's point.

## No Node Replacement

T-PDD is one answer to the note [HDD]'s chapter closed on---the hierarchy
and the statistics now share one model for the whole run---but it inherits
[HDD]'s other limitation untouched: it only ever *deletes*.

The run above shows it directly. Three reducers bottom out at the same wall:

```c
int main() { if (c1) { if (c2) { if (c3) { crash(); } } } }
```

Only [Perses], with its replacement move, reaches:

```c
int main() { crash(); }
```

Combining the two ideas---a whole-tree probabilistic ranking *and* a replacement
move---is a natural next step.

[DDMin]: https://dl.acm.org/doi/10.1109/32.988498
[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[WDD]: https://dl.acm.org/doi/10.1109/ICSE55347.2025.00071
[ProbDD]: https://dl.acm.org/doi/10.1145/3468264.3468625
[T-PDD]: https://ieeexplore.ieee.org/document/10299940
