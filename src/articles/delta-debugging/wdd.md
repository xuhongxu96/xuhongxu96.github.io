# Weighted Delta Debugging

[HDD] hands each level's subtrees to a list-minimizer like [DDMin]. But look at
what those subtrees are: at the top level of a program, one function might be a
thousand-token monster while its neighbor is a one-line `typedef`. DDMin can't
see that. It partitions the list by **count**---equal *number* of elements per
chunk---as if every element were the same size.

That blind spot costs tests. A bigger fragment is statistically more likely to
contain the failure, and so *less* likely to be removable. By splitting purely on
count, DDMin keeps bundling the big, bug-holding element together with small ones
and trying to delete the whole chunk---tests that were never going to succeed.

[WDD] fixes this with one idea: give each element a **weight** equal to its size,
and partition by weight instead of count. Each chunk then carries roughly the
same total size, so the heavy element gets isolated early and DDMin stops wasting
tests trying to remove it in a bundle.

## Weight Is Just Size

Inside HDD the elements are subtrees, and the natural size of a subtree is how
many leaves (atomic units) it contains. Precompute that once for every node:

```rust,ignore
{{#include wdd.rs:weight}}
```

## Partition by Weight, Not Count

Here is DDMin's `partition`---it sorts the elements and cuts them into `n`
chunks of roughly equal *count*:

```rust,ignore
{{#include wdd.rs:partition}}
```

WDD's `weighted_partition` makes the same kind of contiguous chunks, but balances
their total *weight*.

```rust,ignore
{{#include wdd.rs:weighted-partition}}
```

> [!NOTE]
> At the finest granularity (`n >= len`) it falls back to one element per
> chunk---singletons---exactly like `partition`. That is what lets the same
> granularity escalation reach 1-minimality, so WDD needs no extra deletion pass.

## WDD Policy

With the weighted split in hand, WDD is literally [DDMin] with `partition`
swapped for `weighted_partition`.
Same `Policy` trait, same `n = 2, 4, 8, ...` escalation:

```rust,ignore
{{#rustdoc_include wdd.rs:wdd}}
```

Because it's a `Policy`, it drops into [HDD] exactly where DDMin and ProbDD did
(see the [HDD page](./hdd.md)).

```rust,ignore
let weight = tree.leaf_counts();
Hdd::new(&tree, 1, || Wdd { weight: &weight })
```

> [!NOTE]
> WDD weights the level's subtrees by their leaf count.
> Since HDD minimizes top-down,
> when WDD runs on a level,
> none of that level's subtrees have been touched yet.
> Therefore every leaf under a surviving subtree is still present,
> and this is why the leaf count works.

## Run It

The input is a program tree with deliberately **uneven** subtrees:

```text
program {
    fn big {                    // 9 leaves — heavy, holds the bug
        block {
            if { guard; crash(); }
            stmt; stmt; stmt;
        }
        stmt; stmt; stmt; stmt;
    }
    fn n2   fn n3   fn n4       // 1 leaf each — light noise
    fn n5   fn n6   fn n7
}
```

`fn big` holds the bug and is large; six sibling functions are one-leaf noise.
The same disparity repeats one level down (a heavy `block` among small
statements), and again (a heavy `if` among statements).

> [!TIP]
> Press play and compare the two runs. Watch HDD+WDD isolate `fn big` on its
> very first test---dropping all six noise functions at once---while HDD+DDMin
> binary-searches the count, peeling the noise off a chunk at a time.

```rust,edition2024
{{#rustdoc_include wdd.rs:main}}
```

Both reach the same minimal program:

```text
program { fn big { block { if { crash() } } } }
```

> [!NOTE]
> How many oracle calls did each take?
>
> 13 (DDMin) vs 8 (WDD).

The gap comes entirely from weight-balanced
partitioning isolating the heavy, bug-holding subtree early.
On real inputs, where size
disparities span thousands of tokens, the saving is far larger.


[DDMin]: https://dl.acm.org/doi/10.1109/32.988498
[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[WDD]: https://dl.acm.org/doi/10.1109/ICSE55347.2025.00071
[ProbDD]: https://dl.acm.org/doi/10.1145/3468264.3468625
