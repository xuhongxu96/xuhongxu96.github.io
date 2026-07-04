# DDMin---The Original Delta Debugging

[DDMin] is the original delta debugging algorithm, and the one that started it all.

## The Model

The goal of delta debugging is to minimize a failing test case.
Specifically, we want a smaller version of the input that still triggers the bug.
"Smaller" means fewer elements, i.e., fewer of some atomic unit:
characters, tokens, lines, etc.

### Atomic Unit

**Atomic Unit** is the smallest piece of the input that can be removed.
Different inputs have different atomic units---characters for one input,
tokens or lines for another---so the framework fixes no concrete type; it
only asks that a unit be cheap to copy, compare, and hash:

```rust,ignore
{{#include ddmin.rs:atomic-unit}}
```

The demo at the end of this chapter minimizes a set of plain numbers, so
its atomic unit is simply `u32`.

### Configuration

A **Configuration** is a subset of the atomic units in the input:
the units you keep in the current iteration of the minimization.
The full input keeps everything, and reduction shrinks the configuration
while preserving the property (e.g., still triggering the bug).

```rust,ignore
{{#include ddmin.rs:configuration}}
```

### Oracle

The property we want to preserve is checked by an **Oracle**.
Given a configuration, the oracle returns a **Verdict**, i.e., whether it still preserves the property.

```rust,ignore
{{#include ddmin.rs:oracle}}
```

## The Loop

The whole framework is one loop: propose removals, keep the first one the
oracle still finds interesting, and repeat. Everything else---what to
propose, when to stop---is delegated:

```rust,ignore
{{#include ddmin.rs:loop}}
```

A **Delta** is a subset of the current configuration that we propose to remove.

Everything algorithm-specific lives in the **Policy**.

```rust,ignore
{{#include ddmin.rs:policy}}
```

For many delta debugging algorithms, including [DDMin],
the default implementation of `on_reduced` is enough.

## DDMin Policy

In [DDMin], the policy is simple: it partitions the configuration into `n` equal-sized chunks, and proposes to keep each chunk in turn, as well as the complement of each chunk.
The granularity `n` starts at 2 and doubles whenever the algorithm fails to make progress.

```rust,ignore
{{#rustdoc_include ddmin.rs:ddmin}}
```

The `partition` utility it relies on:

> [!TIP]
> Press play to see how it chunks `{1, ..., 8}` as the granularity `n` grows:
> the chunks stay contiguous and as even as possible.

```rust,edition2024
# use std::collections::HashSet;
# trait AtomicUnit: Copy + Eq + std::hash::Hash + Ord {}
# impl<T: Copy + Eq + std::hash::Hash + Ord> AtomicUnit for T {}
# type Configuration<U> = HashSet<U>;
# type Delta<U> = HashSet<U>;
{{#include ddmin.rs:partition}}
#
# fn main() {
#     let config: Configuration<u32> = (1..=8).collect();
#     for n in [2, 3, 4] {
#         let chunks: Vec<Vec<u32>> = partition(&config, n)
#             .iter()
#             .map(|s| {
#                 let mut v: Vec<u32> = s.iter().copied().collect();
#                 v.sort_unstable();
#                 v
#             })
#             .collect();
#         println!("partition({{1..=8}}, {n}) = {chunks:?}");
#     }
# }
```

### Run It

Now, let's see how DDMin works on a simple example.

> [!TIP]
> Press the play button to run the full minimization and watch DDMin
> narrow `{1, ..., 8}` down to `{2, 7}`, probing coarse-to-fine the whole way.

```rust,edition2024
{{#rustdoc_include ddmin.rs:main}}
```

[DDMin]: https://dl.acm.org/doi/10.1109/32.988498
[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[WDD]: https://dl.acm.org/doi/10.1109/ICSE55347.2025.00071
[ProbDD]: https://dl.acm.org/doi/10.1145/3468264.3468625
