---
name: big-o
description: How to reason about time and space complexity in Aldwin — Big-O notation, the rules for deriving it, the reference costs of every common data structure and sort, the hidden costs of Rust std calls, and when complexity matters here and when the simplest code wins. Use when writing or reviewing any loop, collection, recursion, cache or code on a hot path (per frame, per keystroke, per streamed token, per file in the workspace). Pairs with the data-structures skill, which says what to choose once the cost is known.
---

# Big-O

Every change states, at least to itself, how its cost grows with its input.
Reference: [bigocheatsheet.com](https://www.bigocheatsheet.com/); the tables
below are its values, with Rust's own types added.

## The notation

- **O(f)** — an upper bound: grows no faster than f. **Ω(f)** — a lower
  bound. **Θ(f)** — both: grows exactly like f.
- **Best, average, worst case** are different functions of the same input
  size. Plan for the worst case unless the input's shape is known and
  enforced (a hash table's O(1) is its average; its worst is O(n)).
- **Amortized** — the average over a sequence of operations. `Vec::push` is
  O(1) amortized: an occasional O(n) reallocation, doubled capacity, paid
  for by the pushes before it. A latency-sensitive path cares about the
  single worst push; a throughput path does not.
- **Space** counts what the algorithm allocates beyond its input — auxiliary
  space — unless stated otherwise. Recursion depth is space: each frame sits
  on the stack.

## The tiers

| Tier | Complexity | Meaning at n = 1 000 000 |
|---|---|---|
| Excellent | O(1), O(log n) | 1, ~20 steps |
| Good | O(n) | one pass |
| Fair | O(n log n) | a sort |
| Bad | O(n²) | 10¹² steps: minutes to never |
| Horrible | O(2ⁿ), O(n!) | unusable beyond n ≈ 30 and n ≈ 12 |

## Deriving it

1. **Name every input and give each its own variable.** Two different
   collections are `a` and `b`, not n: a loop over files inside a loop over
   comments is O(f · c), and O(n²) would hide which one to shrink.
2. **Sequential steps add; nested steps multiply.** A loop then a loop is
   O(a + b); a loop inside a loop is O(a · b).
3. **Drop constants.** O(2n) is O(n); O(500) is O(1). Constants matter to
   the stopwatch, not the growth: two passes over a `Vec` can beat one pass
   over a `LinkedList`.
4. **Drop non-dominant terms.** O(n² + n) is O(n²); O(n + log n) is O(n).
   Only when the terms share one variable: O(a² + b) stays as it is.
5. **Count the hidden loops.** A call inside a loop costs its own complexity
   each time round. `v.contains(x)` inside `for x in w` is O(v · w).
6. **Recursion** costs (calls made) × (work per call). A call that branches
   twice per level to depth n is O(2ⁿ) — naive Fibonacci. Its space is the
   maximum depth, not the number of calls.
7. **Say what n is.** "O(n)" means nothing until n is named: bytes of a
   file, lines of a diff, entries in the transcript, files in the workspace.

What costs time: operations, comparisons, loops, calls to functions that
loop. What costs space: new variables, new collections, allocations
(`clone`, `collect`, `to_string`, `format!`), and stack frames.

## Data structure operations

| Structure | Rust | Access | Search | Insert | Delete | Space |
|---|---|---|---|---|---|---|
| Array | `[T; N]`, `Vec<T>` | O(1) | O(n) | O(n); push O(1) am. | O(n); pop O(1) | O(n) |
| Stack | `Vec<T>` | O(n) | O(n) | O(1) am. | O(1) | O(n) |
| Queue / deque | `VecDeque<T>` | O(1) by index | O(n) | O(1) am. at either end | O(1) at either end | O(n) |
| Singly / doubly linked list | `LinkedList<T>` | O(n) | O(n) | O(1) at a held node | O(1) at a held node | O(n) |
| Skip list | — | avg O(log n), worst O(n) | same | same | same | O(n log n) |
| Hash table | `HashMap`, `HashSet` | — | avg O(1), worst O(n) | avg O(1), worst O(n) | avg O(1), worst O(n) | O(n) |
| Binary search tree (unbalanced) | — | avg O(log n), worst O(n) | same | same | same | O(n) |
| Cartesian tree | — | — | avg O(log n), worst O(n) | same | same | O(n) |
| B-tree | `BTreeMap`, `BTreeSet` | O(log n) | O(log n) | O(log n) | O(log n) | O(n) |
| Red-black / AVL tree | — (use `BTreeMap`) | O(log n) | O(log n) | O(log n) | O(log n) | O(n) |
| Splay tree | — | — | O(log n) am. | O(log n) am. | O(log n) am. | O(n) |
| KD tree | — | avg O(log n), worst O(n) | same | same | same | O(n) |
| Binary heap | `BinaryHeap<T>` | peek O(1) | O(n) | O(1) avg, O(log n) worst | pop O(log n) | O(n) |
| Sorted `Vec` | `Vec<T>` + `binary_search` | O(1) | O(log n) | O(n) | O(n) | O(n) |

The linked list's O(1) insert and delete assume the node is already in hand;
finding it first is O(n), so in practice both are O(n). A stack's peek is
O(1) (`last()`), whatever some references say.

## Sorting

| Algorithm | Best | Average | Worst | Space |
|---|---|---|---|---|
| Quicksort | n log n | n log n | n² | log n |
| Mergesort | n log n | n log n | n log n | n |
| Timsort | n | n log n | n log n | n |
| Heapsort | n log n | n log n | n log n | 1 |
| Bubble sort | n | n² | n² | 1 |
| Insertion sort | n | n² | n² | 1 |
| Selection sort | n² | n² | n² | 1 |
| Tree sort | n log n | n log n | n² | n |
| Shell sort | n log n | n (log n)² | n (log n)² | 1 |
| Bucket sort | n + k | n + k | n² | n |
| Radix sort | nk | nk | nk | n + k |
| Counting sort | n + k | n + k | n + k | k |
| Cubesort | n | n log n | n log n | n |

All times are O(·). A comparison sort cannot beat O(n log n) in the worst
case; counting, radix and bucket sorts can, because they read the keys
instead of comparing them — k is the key range or digit count. A stable
sort keeps equal elements in input order: mergesort, Timsort, insertion and
counting sort are stable; quicksort, heapsort and selection sort are not.

In Rust, never hand-roll a sort. `slice::sort` / `sort_by_key` are stable,
O(n log n) worst, O(n) best on already-sorted runs, and allocate O(n).
`sort_unstable` is O(n log n) worst, in place, and faster when equal
elements need no order. `sort_by_cached_key` computes an expensive key once
per element instead of O(n log n) times. `select_nth_unstable` finds the
k-th element in O(n) without sorting.

## Hidden costs in std

| Call | Cost | Instead, when it runs in a loop |
|---|---|---|
| `slice.contains(x)`, `iter().find`, `iter().position` | O(n) | a `HashSet` / `HashMap`, or `binary_search` on sorted data |
| `vec.remove(i)`, `vec.insert(i, x)` | O(n − i): shifts the tail | `swap_remove` if order is free; `VecDeque` for the front; `retain` for many removals |
| `vec.remove(0)` in a loop | O(n²) total | `VecDeque::pop_front`, or iterate and `drain(..)` |
| `string.insert(0, …)`, `s = format!("{x}{s}")` | O(len) each | build forwards, or collect parts and `concat` / `join` once |
| `s.chars().nth(i)`, `s.chars().count()` | O(i), O(len) | `str::len` is bytes in O(1); index by byte offsets from `char_indices` |
| `clone()` of `Vec`, `String`, `HashMap` | O(n) time and space | borrow; `Arc` for shared ownership |
| `collect()` just to iterate again | O(n) allocation | keep the iterator chain |
| `BTreeMap::range`, `first_key_value` | O(log n) | — already the fast path |
| `push` n times into `Vec::new()` | log n reallocations | `Vec::with_capacity(n)` when n is known |

## When it matters here

Complexity is a judgment about how large n gets and how often the code
runs. **The simplest correct code wins when n is small and bounded** (quality-gate:
"going above and beyond means making it lean and simple"): a linear scan over the tool registry, the slash
commands, the MCP servers or a theme's palette is right, and a `HashMap` or a
sorted index there is complexity with no return.

It matters when either factor is unbounded:

- **Frequency.** A frame is drawn every 100 ms tick while anything animates
  (`crates/tui/src/run.rs`, `run_loop`) and on every streamed token. Work done per
  frame is multiplied by both. Anything proportional to the whole
  conversation, the whole diff or the whole file must not run per frame.
- **Size that grows with use.** The transcript grows for the whole session
  and `/resume` reloads it; a file the model reads can be megabytes; a
  workspace can hold a hundred thousand files; a diff can be thousands of
  lines. Assume these are large.

**The worked example is the transcript** (`crates/tui/src/ui/transcript.rs`).
Re-rendering every entry per streamed token was O(entries × rows) per frame
and cost 58% of a core at four turns. The fix is two techniques from the
data-structures skill: rendered rows are cached per entry and rebuilt only
when the entry differs (memoization, O(changed) per frame), and `starts`
holds prefix sums of row counts so `viewport` finds the first visible block
with `partition_point` — O(log entries + height) instead of O(total rows).

## Checking a change

1. For each loop, recursion and collection in the diff: name n, write its
   time and space, and multiply by how often the code runs.
2. Is any n unbounded in use? If the answer is worse than O(n log n) in it,
   or O(n) in it on a per-frame path, find the better structure (the
   data-structures skill) or bound n and say where it is bounded.
3. Is any n small and fixed? Then prefer the plainest code, whatever its
   Big-O.
4. Unsure? Measure — a test with a large input, or a timing around the hot
   path — before adding structure. Theory says where to look; the
   measurement decides.

`/review` holds a change to this skill: stage 2's Clippy lints catch the
mechanical part (`stable_sort_primitive`, `large_stack_arrays`,
`inefficient_to_string`, and the default `perf` group), and the code judge
(stage 6) the rest, through quality-gate §5.
