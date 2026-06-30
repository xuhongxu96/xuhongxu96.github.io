# Delta Debugging: A Unified Perspective

> [!WARNING]
> This series is still a work in progress.

If you've spent time minimizing failing test cases,
you've probably met a small zoo of algorithms:

- **[DDMin]**, the original delta debugging minimizer;
- **[HDD]**, hierarchical delta debugging, which runs DDMin over a parse tree;
- **[Perses]**, which exploits the grammar more aggressively;
- **[ProbDD]**, probabilistic delta debugging, which puts a probability model over what to remove; and
- **[WDD]**, weighted delta debugging, which weights elements by size so that partitioning treats a big chunk differently from a tiny one.

Based on how the papers themselves position these algorithms,
they do not sit at the same level.

- _DDMin_, _ProbDD_, and _WDD_ are **list-minimizers**---they take a flat
  list and remove the irrelevant elements; - _ProbDD_ and _WDD_ are refinements of DDMin that partition more cleverly, but still only ever see a list.
- _HDD_ and _Perses_ are **tree-drivers**---they walk a parse tree and,
  at each level or group of repeatable nodes, hand a list of children
  down to a **list-minimizer**.
  - _DDMin_ is a component that _HDD_ and _Perses_ call;
  - _ProbDD_ and _WDD_ are drop-in replacements for this component.

This layering has a cost.
Because the probability model in ProbDD and the weighting in WDD are only applied to the list-minimizer,
they can only inform decisions within one flat list---they
cannot reach across the tree.
The hierarchy and the statistics never talk.

In this series, I want to argue that this split is an accident of
how these algorithms were built, not something fundamental.
We can make all five algorithms become pluggable components at a single level.

> [!NOTE]
> This series mainly focuses on _program reduction_,
> a subarea of delta debugging that deals with structured inputs
> that can be described by a grammar.

> [!TIP]
> Checkout [Program Reduction 101](https://yqtian-se.github.io/program-reduction-101/) for a comprehensive tutorial on program reduction, including a gentle introduction to delta debugging.

## Dealt a Debugging with Delta Debugging

You hit a bug that makes the compiler break,<br>
With forty thousand lines of code at stake.<br>
Somewhere inside, ten crucial lines reside,<br>
While all the rest is noise to cast aside.

Your task is finding where the secret lies,<br>
For massive files will only strain the eyes.<br>
So you remove a chunk and run anew,<br>
And ask: does this old bug still trigger, too?

If yes, the chunk was noise—toss it away.<br>
If no, put it right back; the code must stay.<br>
You keep on cutting, shrinking every pass,<br>
Eliminating all the bloated mass.

Until no single piece can be erased,<br>
Without the bug itself being displaced.<br>
From forty thousand lines to merely ten,<br>
The smallest breaking proof is captured then.

Stripped of the noise, the naked truth avails:<br>
Just this, and nothing else—right here, it fails.

--- Claude Opus and Gemini Pro, 2026

[DDMin]: https://dl.acm.org/doi/10.1109/32.988498
[HDD]: https://dl.acm.org/doi/10.1145/1134285.1134307
[Perses]: https://dl.acm.org/doi/10.1145/3180155.3180236
[WDD]: https://dl.acm.org/doi/10.1109/ICSE55347.2025.00071
[ProbDD]: https://dl.acm.org/doi/10.1145/3468264.3468625
