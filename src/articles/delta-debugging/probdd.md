# Probabilistic Delta Debugging

[DDMin] follows a fixed script: partition into n chunks, test each chunk and
its complement, double n, and repeat. Test outcomes are only used to guide
the scripted steps---the oracle's signal is otherwise ignored.

[ProbDD] maintains a probability model over units, estimating how likely each
unit is to be essential (i.e., to survive into the minimized result). It
updates that model after every test and, instead of following a preset
schedule, chooses at each step the removal expected to produce the largest
reduction. Failed removals then inform the model to avoid similar attempts.

## A Probability Model

Give every unit a probability that it is **essential**---that it survives into
the minimized result. Everything starts at a small prior `p0`.

```rust,ignore
{{#include probdd.rs:model}}
```

## Choosing What To Remove

Removing a set of units only succeeds if *every* unit in it is non-essential.
Suppose we remove $k$ units with probabilities $p_1, ..., p_k$.
The probability of success is $\prod_{i=1}^k (1 - p_i)$,
so the expected number of units a removal will delete is
$k \cdot \prod_{i=1}^k (1 - p_i)$.

ProbDD sorts units by probability and removes the prefix that maximizes that gain.

```rust,ignore
{{#include probdd.rs:choose}}
```

## Learning From Failure

When a removal *fails*, at least one of those units was essential after all.
[ProbDD] raises their probabilities with a Bayesian update.
A removal of a single unit that fails is conclusive---that unit is
essential, and its probability jumps straight to `1`.

> [!TIP]
> Press play to watch three units start at the prior `0.1`, rise after a failed
> bulk removal, then watch unit 2 get pinned to `1.000` the moment removing it
> *alone* fails.

```rust,edition2024
# use std::collections::HashMap;
# type AtomicUnit = u32;
{{#include probdd.rs:update}}
#
# fn show(probs: &HashMap<AtomicUnit, f64>) -> Vec<String> {
#     let mut v: Vec<(AtomicUnit, f64)> =
#         probs.iter().map(|(&u, &p)| (u, p)).collect();
#     v.sort_by_key(|&(u, _)| u);
#     v.iter().map(|(u, p)| format!("{u}:{p:.3}")).collect()
# }
#
# fn main() {
#     let mut probs: HashMap<AtomicUnit, f64> =
#         [(1, 0.1), (2, 0.1), (3, 0.1)].into_iter().collect();
#     println!("prior:                 {:?}", show(&probs));
#     bayes_update(&mut probs, &[1, 2, 3]);
#     println!("after {{1,2,3}} fails:   {:?}", show(&probs));
#     bayes_update(&mut probs, &[2, 3]);
#     println!("after {{2,3}} fails:     {:?}", show(&probs));
#     bayes_update(&mut probs, &[2]);
#     println!("after {{2}} alone fails: {:?}", show(&probs));
# }
```

## ProbDD Policy

Let's look back at the delta debugging loop:

```rust,ignore
{{#include ddmin.rs:loop}}
```

If the current candidate removal fails,
the loop iterates the next candidate and tries again;
otherwise, it breaks and calls `propose` again with the new configuration.
Therefore, the act of iterating to the next candidate is the "that removal failed" signal that ProbDD needs to update its model.

Now, let's implement the new policy for ProbDD:

```rust,ignore
{{#include probdd.rs:probdd}}
```

> [!NOTE]
> Unlike DDMin, ProbDD does not march down to singleton chunks on a schedule, so
> it guarantees only *conditional* 1-minimality (1-minimal when the units are
> independent).
> However, our loop above guarantees that a fixpoint is reached,
> so the result is 1-minimal.

### Run It

Now, let's see how ProbDD works on the same example as DDMin.

> [!TIP]
> Press play to minimize the same `{1, ..., 8}` problem as the [DDMin] page
> ("interesting iff it keeps 2 and 7").

```rust,edition2024
{{#rustdoc_include probdd.rs:main}}
```

> [!TIP]
> Watch the first probe try to delete *everything*,
> then watch the removals shrink as failures push
> probabilities up.

> [!NOTE]
> How many oracle calls did DDMin and ProbDD make respectively?
>
> 33 v.s. 12.

[DDMin]: https://dl.acm.org/doi/10.1109/32.988498
[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[WDD]: https://dl.acm.org/doi/10.1109/ICSE55347.2025.00071
[ProbDD]: https://dl.acm.org/doi/10.1145/3468264.3468625
