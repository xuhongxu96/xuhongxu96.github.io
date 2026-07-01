# Hands-On Delta Debugging in Rust

If you've spent time minimizing failing test cases,
you've probably met a small zoo of algorithms:

- **[DDMin]**, the original delta debugging minimizer;
- **[ProbDD]**, probabilistic delta debugging, which puts a probability model over what to remove; and
- **[HDD]**, hierarchical delta debugging, which runs DDMin over a parse tree;
- **[WDD]**, weighted delta debugging, which weights elements by size so that partitioning treats a big chunk differently from a tiny one.
- **[Perses]**, which exploits the grammar more aggressively;

This series builds each of them from scratch in Rust.

Rather than unrelated implementations, they turn out to share one shape:
a single `reduce` loop that repeatedly proposes a deletion and tests it
against an oracle, plus a swappable `Policy` that decides what to propose
next.

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
