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

T-PDD reuses the exact same parse tree
Perses built (`Kind`, `Node`, `Tree`, `live`, `leaves_under`)---see the
[Perses page](./perses.md) if you haven't read it---and turns that
structural fact into a probability.

> [!CAUTION]
> The [T-PDD] paper and its official artifact don't fully agree: the shipped code
> adds node replacement, and tuning constants the
> paper never mentions. This chapter follows the paper's model, not the
> artifact.

## A Prior From Tree

Every present node other than the root gets a *conditional* retention
probability:

$$
p_n = P(n\text{ survives} \mid n\text{'s parent survives})
$$

The `Tree`'s own
`deletable` check (see [Perses](./perses.md#a-parse-tree-by-hand))
turns out to be exactly the boolean the prior needs: a
`List` element gets the hyperparameter $\sigma$; everything else is
mandatory, $p = 1$.

```rust,ignore
{{#include t-pdd.rs:priors}}
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

$\text{pass}(d)$ alone doesn't measure how much a deletion is worth:
removing one leaf at 99% matters less than removing a hundred at 90%. Weight
it by leaf count for the **expected gain**:

$$
\text{gain}(d) = |\text{leaves\_under}(d)| \cdot \text{pass}(d)
$$

```rust,ignore
{{#include t-pdd.rs:expected-gain}}
```

> [!NOTE]
> A mandatory node---a `Block`'s `{` and `}`, a `Call`'s function name and
> parentheses---always has at least one mandatory token as a child, and a
> mandatory token's own $q$ is $1 - 1 = 0$. That $0$ cascades straight up:
> every mandatory node's expected gain is $0$.

## Picking the Best Candidate

Like [ProbDD]'s `best_prefix`, sort by the score---here, expected gain,
descending.

```rust,ignore
{{#include t-pdd.rs:choose}}
```

> [!NOTE]
> Unlike [ProbDD]'s `best_prefix`, which can combine leaves from anywhere in
> the tree, a T-PDD candidate is always one node's whole subtree---never a
> combination across subtrees.

> [!WARNING]
> We deliberately diverge from the paper on the threshold: `0.0`, not the
> paper's `1.0`. The paper's cutoff is tuned for speed on huge inputs. On
> this article's five-statement example below,
> a cutoff of `1.0` would stop early,
> leaving noise still in the result.

## Learning From Failure

When a candidate $d$ fails,
at least one leaf under it was essential after all.
So the belief $p_d$ must be updated (raised) to reflect that:

$$
p_d \leftarrow \frac{p_d}{1 - \text{pass}(d)}
$$

```rust,ignore
{{#include t-pdd.rs:update}}
```

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

Some bugs need more than one thing to survive together. Suppose the crash
only reproduces with a companion `setup()` call still in place---drop
either one alone and the bug hides.

```c
int main() {
   noise1(); crash(); noise2(); setup(); noise3(); noise4();
}
```

All four reducers share the same oracle---interesting iff the program still
contains *both* `crash` and `setup`, and still parses:

```rust,ignore
{{#include t-pdd.rs:make-oracle}}
```

We compare **HDD**, **HDD+ProbDD**, **Perses**, and **T-PDD**.

> [!TIP]
> Press play and watch each reducer work through the scattered noise.
>
> Perses's replacement search finds nothing to promote in this tree---no
> redundant nesting, no mandatory node with a compatible descendant---so it
> degrades to plain list-element deletion.
> In this case, Perses works identically to HDD.
>
> T-PDD has no replacement move either, but never needed
> one: watch it walk through `noise1`...`noise4`, `crash`, and `setup` in
> exactly six calls, one test per statement.

```rust,edition2024
{{#rustdoc_include t-pdd.rs:main}}
```

```text
HDD        => 32 calls
HDD+ProbDD => 17 calls
Perses     => 32 calls
T-PDD      => 6 calls
```

> [!NOTE]
> For this example's flat tree input, T-PDD just needs one oracle test per
> statement.
>
> HDD and Perses need far more (32) because
> `DDMin`'s binary partitioning tests *batches*, and since the noise sits
> before, between, and after `crash`/`setup`, most batches contain both an
> essential and a removable element, making the batch fail.
>
> HDD+ProbDD (17) splits the difference---probability-guided batching beats
> blind bisection---but it still tests multiple units per call and raises
> all of their beliefs together on failure.

## No Node Replacement

T-PDD closes the gap [HDD]'s closing note pointed at---the hierarchy and
the statistics now share one model for the whole run---but it inherits
[HDD]'s other limitation untouched: it only ever *deletes*.

[Perses] promotes a compatible descendant into a mandatory node's place.
The nested `if` example on the [Perses page](./perses.md) stays
out of reach for T-PDD.

Combining the two ideas---a whole-tree probabilistic ranking *and* a replacement
move---is a natural next step.

[DDMin]: https://dl.acm.org/doi/10.1109/32.988498
[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[WDD]: https://dl.acm.org/doi/10.1109/ICSE55347.2025.00071
[ProbDD]: https://dl.acm.org/doi/10.1145/3468264.3468625
[T-PDD]: https://ieeexplore.ieee.org/document/10299940
