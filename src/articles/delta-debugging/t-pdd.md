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

T-PDD reuses the parse-tree machinery Perses built (`Kind`, `NodeId`,
`Node`, `Tree`, `live`, `leaves_under`)---see the
[Perses page](./perses.md) if you haven't read it---and turns that
structural fact into a probability.

The running example is new, though, and picked to make memory matter.
The nested-`if` input of the last two chapters is the *easy* case: its
essential part is one contiguous spine, so a reducer loses little by
forgetting. Real failure-inducing inputs are rarely that polite---a
reproduction typically needs several cooperating statements *far apart*
in the file: some setup, a state change, and only then the crash site.
This chapter's bug takes three, each at a different level of the tree:

```c
int main() {
   setup();
   if (c1) {
      corrupt();
      if (c2) {
         crash();
         noise();
      }
      noise();
   }
   noise();
}
```

Interesting means: `setup()`, `corrupt()`, *and* `crash()` all survive, and
the program still parses.

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

On this chapter's tree (with $\sigma = 0.5$; mandatory wrapper
tokens and blocks elided from the drawing, all at prior $1.0$):

```text
func                                  (root -- no prior)
└─ { stmt* }                          1.0
   ├─ setup();                        0.5   <- List elements
   ├─ if (c1) { stmt* }               0.5
   │  ├─ corrupt();                   0.5
   │  ├─ if (c2) { stmt* }            0.5
   │  │  ├─ crash();                  0.5
   │  │  └─ noise();                  0.5
   │  └─ noise();                     0.5
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
climbs through both `if`s (the mandatory lists and blocks in between
have $p = 1$ and leave $P$ unchanged):

$$
0.5
\;\xrightarrow{\;\text{if}(c2)\;}\; 0.75
\;\xrightarrow{\;\text{if}(c1)\;}\; 0.875
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

Here is the full first-pass ranking on the demo tree:

| candidate (`List` element) | tokens | $\text{pass}$ | gain |
|----------------------------|--------|---------------|------|
| `if (c1) …`                | 28     | 0.5           | **14** |
| `if (c2) …`                | 14     | 0.75          | 10.5 |
| `crash();`                 | 4      | 0.875         | 3.5  |
| `noise();` (innermost)     | 4      | 0.875         | 3.5  |
| `corrupt();`               | 4      | 0.75          | 3    |
| `noise();` (in `c1`'s block) | 4    | 0.75          | 3    |
| `setup();`                 | 4      | 0.5           | 2    |
| `noise();` (top level)     | 4      | 0.5           | 2    |

Two forces set this order. Sheer mass puts `if (c1)` on top: 28 tokens
outweigh its low $\text{pass}$. And among the equal-sized calls, only
wrapping depth differentiates: `crash();` under two optional `if`s
prices at 0.875, `corrupt();` under one at 0.75, `setup();` under none at
0.5. The model weighs *how much would go* against *how plausibly it can
go*---and notice it knows *shape*, not *content*: the essential
`crash();` outranks every harmless `noise();`. The prior is allowed to
be wrong; failures are about to correct it.

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

Concretely: the first pick, `if (c1)`, fails---it holds two of the three
essentials. Its belief becomes $0.5 / (1 - 0.5) = 1$---**pinned**. And
the pin *propagates* through every later $\text{pass}$ computation: with
`if (c1)` certain to survive, the escape route through it closes, so
`if (c2)`'s $\text{pass}$ drops from 0.75 to 0.5; when `if (c2)` fails
next and pins in turn, `crash();` and the innermost `noise();` fall from
0.875 to 0.5. Nothing under a fully-pinned spine is ever retried; the
knowledge from one failure prices every future candidate.

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

Here is the input again, with its two structural facts called out:
**every level holds one essential** (`setup()` at depth 0, `corrupt()` at
depth 1, `crash()` at depth 2), and therefore **every big subtree is
poisoned**---`if (c1)` holds two essentials, `if (c2)` one, `main`'s
body all three. The only safely deletable things are the three lone
`noise();` calls:

```c
int main() {
   setup();            // essential, depth 0
   if (c1) {           // poisoned: holds corrupt() AND crash()
      corrupt();       // essential, depth 1
      if (c2) {        // poisoned: holds crash()
         crash();      // essential, depth 2
         noise();
      }
      noise();
   }
   noise();
}
```

Every reducer runs against the same oracle:

```rust,ignore
{{#include t-pdd.rs:make-oracle}}
```

We compare **HDD**, **HDD+ProbDD**, **Perses**, **Perses+ProbDD**, and
**T-PDD**.

> [!TIP]
> Press play and watch the belief map earn its keep: five failures, each
> pinning one belief---`if (c1)`, `if (c2)`, `crash();`, `corrupt();`,
> `setup();`---and three successes deleting the three `noise();` calls.
> Eight tests, none of them asking a question the model already
> answered.

```rust,edition2024
{{#rustdoc_include t-pdd.rs:main}}
```

```text
HDD           => 12 calls
HDD+ProbDD    => 13 calls
Perses        => 64 calls
Perses+ProbDD => 63 calls
T-PDD         => 8 calls
```
<!-- kept in sync with the asserts in t-pdd.rs main -->

Each reducer's result and bill follow from those two structural facts:

- **HDD (12 calls)** batches siblings level by level, and any batch that
  spans an essential fails. Its log shows the forgetting outright: the
  same doomed candidate---`int main() { setup(); }`, everything else
  deleted---is tested *twice*, because after each success DDMin starts
  its granularity walk over and nothing remembers the earlier failure.
- **HDD+ProbDD (13 calls)** carries the model that would prevent exactly
  those repeats---but it is rebuilt at every level, so the learning cost
  is re-paid three times and never amortizes. On an input this small the
  model costs one call *more* than plain HDD.
- **Perses (64 calls)** is the one reducer whose *final program* is
  smaller: only its replacement move can strip the `if` wrappers, which
  pure deleters must keep. But every big subtree being poisoned makes
  replacements a minefield: replacing `if (c1)` (or `main`'s body) by
  any single descendant amputates `corrupt()` or `crash()`, so nearly the
  whole candidate list is doomed---and, having no memory, Perses
  re-derives and re-tests that same doomed list on *every pass*.
- **Perses+ProbDD (63 calls)** pins the blame precisely: swapping the
  inner deleter for the probabilistic one saves almost nothing, because
  the waste never was in list deletion---it is in the replacement loop,
  which consults no model at all.
- **T-PDD (8 calls)** pays for each structural fact exactly once: each
  poisoned subtree fails one test and is pinned, the pin re-prices
  everything beneath it, and the three `noise();` calls then fall in
  three tests. Scattered essentials cost every forgetful reducer per
  level or per pass; a whole-tree memory pays per *fact*.

## No Node Replacement

T-PDD is one answer to the note [HDD]'s chapter closed on---the hierarchy
and the statistics now share one model for the whole run---but it inherits
[HDD]'s other limitation untouched: it only ever *deletes*.

The run above shows it directly. The three deletion-only reducers all
bottom out at the same wall, the `if` wrappers intact:

```c
int main() { setup(); if (c1) { corrupt(); if (c2) { crash(); } } }
```

Only [Perses], with its replacement move, strips them:

```c
int main() { setup(); { corrupt(); crash(); } }
```

---but at 64 oracle calls (63 with ProbDD as its inner deleter) against
T-PDD's 8, each extra call a forgotten failure, re-tested. A stronger
move set is worth little without a memory to aim it, and a sharp memory
cannot reach what its moves cannot express.

Combining the two ideas---a whole-tree probabilistic ranking *and* a replacement
move---is a natural next step.

[DDMin]: https://dl.acm.org/doi/10.1109/32.988498
[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[WDD]: https://dl.acm.org/doi/10.1109/ICSE55347.2025.00071
[ProbDD]: https://dl.acm.org/doi/10.1145/3468264.3468625
[T-PDD]: https://ieeexplore.ieee.org/document/10299940
