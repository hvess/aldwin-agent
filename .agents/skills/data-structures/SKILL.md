---
name: data-structures
description: How to choose the data structure and algorithm for a job in Aldwin — each structure's strengths and weaknesses mapped to its Rust std type, a table from the operation you need to the type that serves it, and the algorithms (sorting, searching, BFS/DFS, shortest paths, recursion, dynamic programming, caching and memoization) with when each is right and the std call that already implements it. Use when adding or changing a collection, a lookup, a traversal, a sort, a cache or a recursive function in crates/. Pairs with the big-o skill, which says how to cost the choice.
---

# Data structures and algorithms

A data structure is chosen for the operations the code performs on it,
most frequent first. Name them, cost them (big-o skill), and take the
simplest std type that serves them. Reference:
[ZTM's data structures and algorithms cheat sheet](https://zerotomastery.io/cheatsheets/data-structures-and-algorithms-cheat-sheet/).

**Canonical before novel** (quality-gate §1): std's collections, sorts and
searches are the implementation. A hand-written linked list, tree, sort or
binary search is a defect unless std has no equivalent, and a crate
(`indexmap`, `petgraph`, a trie) is an architectural decision, not a detail.

## Choosing by operation

| You need | Use | Not |
|---|---|---|
| Store and iterate in order; push/pop at the end | `Vec<T>` | anything else — the default |
| A fixed, known-at-compile-time set | an array, a `const` slice, or an `enum` | a map built at startup |
| Membership or lookup by key, order irrelevant | `HashSet` / `HashMap` | `Vec::contains` in a loop |
| Lookup by key **and** iteration in key order, or range queries | `BTreeMap` / `BTreeSet` | a `HashMap` sorted on every read |
| Deterministic output: a snapshot, a prompt, a file written, a list shown | `BTreeMap` / `BTreeSet`, or a `Vec` in insertion order | `HashMap` — its iteration order changes run to run |
| FIFO queue, or push/pop at both ends | `VecDeque<T>` | `Vec::remove(0)` |
| LIFO stack | `Vec<T>` (`push`, `pop`, `last`) | `VecDeque` |
| Always take the smallest or largest next | `BinaryHeap<T>` (`Reverse` for a min-heap) | re-sorting a `Vec` per take |
| Search a large collection that rarely changes | a sorted `Vec` + `binary_search` / `partition_point` | a tree |
| Find the row, byte or item at an offset over variable-length parts | prefix sums in a `Vec<usize>` + `partition_point` | summing from the start each time |
| Shared read-only data across tasks | `Arc<T>` | cloning it per task |
| Shared mutable state across tasks | `Arc<Mutex<T>>` owned by the composition root (quality-gate §5) | a global |
| Words sharing prefixes, prefix queries | `BTreeSet<String>` + `range(prefix..)` | a hand-written trie |
| A graph | adjacency list: `Vec<Vec<usize>>`, or `HashMap<Id, Vec<Id>>` | an adjacency matrix, unless dense and small |

This codebase's choices, as models: the staged changeset is a
`BTreeMap<PathBuf, Staged>` so the review lists files in path order
(`crates/tools/src/staging.rs`); the tool registry is a `HashMap` because
only lookup by name matters (`crates/tools/src/registry.rs`); settings maps
written to YAML are `BTreeMap` so the file is stable across writes
(`crates/config/src/domain.rs`).

## The structures

**Array — `Vec<T>`, `[T; N]`, `&[T]`.** Contiguous memory: O(1) index,
O(1) amortized push and pop at the end, the smallest footprint and the best
cache locality of any structure. Slow at inserting or removing anywhere but
the end — every later element shifts. Fixed size only as `[T; N]`. The
default: reach for something else only when a named operation needs it.

**Hash table — `HashMap<K, V>`, `HashSet<T>`.** A hash function maps the key
to a slot: average O(1) search, insert and delete; O(n) worst case when
keys collide. std resolves collisions itself and resizes as it fills, and
its default SipHash hasher resists keys chosen to collide — keep it unless
the keys are trusted and a profile says hashing is the cost. Unordered, and
iterating all keys walks the whole table. Keys must be `Hash + Eq`.

**Linked list — `LinkedList<T>`.** Nodes scattered in memory, each pointing
to the next (and previous, doubly linked). O(1) insert and delete at a node
you already hold; O(n) to find it; poor cache locality and a pointer or two
of overhead per element. **Almost never the answer in Rust:** `Vec` or
`VecDeque` beats it on every common workload. Use it only for O(1)
splicing of whole lists (`append`, `split_off`) and say why.

**Stack and queue — `Vec<T>`, `VecDeque<T>`.** A restricted interface over
an array: push, pop and peek at one end (stack, LIFO) or add at the back
and take from the front (queue, FIFO), all O(1). The restriction is the
point — it states how the data is used. `VecDeque` is a ring buffer, so
it also indexes in O(1).

**Tree.** A hierarchy: a root, nodes with children, leaves, no cycles.
Model a domain tree (a parsed document, a directory) as an enum or struct
owning `Vec<Child>`; ownership is the tree, so no pointers are needed.

**Binary search tree, AVL, red-black — `BTreeMap`, `BTreeSet`.** Left is
smaller, right is larger: O(log n) search, insert and delete while
balanced, O(n) when it degenerates into a list. std's B-tree is always
balanced and cache-friendly. Choose it over a hash table when order
matters — sorted iteration, `range`, `first_key_value`, `last_key_value` —
and accept O(log n) for every operation.

**Binary heap, priority queue — `BinaryHeap<T>`.** A complete binary tree
where each parent outranks its children; siblings are unordered. O(1) peek
at the top, O(log n) pop, O(1) average push. A max-heap; wrap items in
`std::cmp::Reverse` for a min-heap. Slow to search. The structure for
"next most urgent", Dijkstra, and top-k (keep a heap of k).

**Trie.** A tree over characters, storing shared prefixes once, answering
prefix queries in O(length of the key). Not in std, and usually larger
than a set of strings: a pointer per link outweighs the shared bytes. A
`BTreeSet<String>` with `range(prefix.to_owned()..)` and `take_while`
answers the same query in O(log n + matches); build a trie only when a
profile says that is not enough.

**Graph.** Vertices joined by edges, with no hierarchy: directed or not,
weighted or not, cyclic or not. Represent it as an adjacency list (each
vertex's neighbours) — O(V + E) space, the right default for sparse
graphs; an adjacency matrix costs O(V²) and suits only small dense ones.
Store vertices in a `Vec` and refer to them by index, not by `Rc` pointers.

## Algorithms

**Sorting.** Use `sort` (stable) or `sort_unstable` (in place, faster when
ties need no order); `sort_by_key`, `sort_by_cached_key` for an expensive
key. Both are O(n log n) worst case and adapt to already-sorted runs. Never
write bubble, selection or insertion sort — O(n²). Sort once and keep the
data sorted, rather than sorting per read. Need only the top k? A
`BinaryHeap` of k, or `select_nth_unstable`, is O(n), not O(n log n).

**Linear search** — `iter().find`, `position`, `contains`: O(n), no
precondition. Right for small or unsorted or constantly changing data.

**Binary search** — `binary_search`, `binary_search_by_key`,
`partition_point`: O(log n), **only on data sorted by the same key**. The
sortedness is an invariant: state it on the field (comments skill) and keep
it on every insert. `partition_point` is the general form: the first index
where a predicate turns false.

**Breadth-first search.** Level by level from a start, with a `VecDeque`
queue and a visited set, marking a node visited when it is enqueued. Finds
the shortest path in an **unweighted** graph. O(V + E) time, and memory for
a whole level at once.

**Depth-first search.** Branch by branch, with a `Vec` stack (or
recursion). Finds whether a path exists, detects cycles, gives a
topological order, and uses less memory than BFS on wide graphs. O(V + E)
time, O(V) space. On a binary tree its orders are pre-order (node, left,
right), in-order (left, node, right — sorted, for a search tree) and
post-order (left, right, node — children before parents, as for freeing or
sizing a directory).

**Shortest paths with weights.** Dijkstra: non-negative weights, a
`BinaryHeap<Reverse<(cost, node)>>`, O((V + E) log V). Bellman-Ford:
handles negative weights by relaxing every edge V − 1 times, O(V · E).
Relaxation is lowering a node's known distance when a shorter path to it
appears.

**Recursion.** A function calling itself on a smaller instance, with a base
case that stops it. It fits problems that divide into smaller identical
subproblems whose answers combine — trees above all. In Rust, **prefer
iteration when the depth depends on input**: there is no guaranteed
tail-call elimination, a spawned thread's stack is 2 MiB by default and a
stack overflow aborts the process. A recursion over a parse tree of the
model's output, a directory walk or a user's file must be an explicit
`Vec` stack instead, or be bounded and say where. Recursion over a small
fixed structure is fine and often clearer.

**Divide and conquer.** Split the problem, solve the parts, combine —
mergesort, quicksort, binary search. It is what makes O(n log n) and
O(log n) possible.

**Dynamic programming.** When a recursive solution solves the same
subproblem many times, store each answer and reuse it: memoize top-down
(a `HashMap` from arguments to result) or fill a table bottom-up (a `Vec`
indexed by subproblem). Naive Fibonacci is O(2ⁿ); memoized it is O(n).

**Caching and memoization.** A cache keeps data that is expensive to get
again; memoization is a cache of a function's results keyed by its inputs.
Every cache needs a stated **invalidation rule** — what change makes an
entry stale — and a bound on its size. The model here is the transcript
(`crates/tui/src/ui/transcript.rs`): rendered rows cached per log entry,
an entry re-rendered only when it differs by `==` from its cached copy, the
whole cache dropped when the width or theme changes, and prefix sums over
the rows so the viewport is found by `partition_point`.

## Before choosing

1. **The data:** how large, how it grows, whether it is sorted, whether its
   order is visible to anyone.
2. **The operations:** which ones, how often, on which path. The most
   frequent operation picks the structure.
3. **The trade-off:** time against memory, and both against the simplest
   code. A small bounded collection takes the plainest structure (big-o
   skill, "When it matters here").
4. **The constraints:** determinism for snapshots and files, `Send + Sync`
   across tasks, no blocking in async code (quality-gate §5).
5. **Verify it.** A test with a large input, or a measurement on the hot
   path, confirms the choice; theory alone does not.

`/review` holds a change to this skill: stage 2 refuses `LinkedList`
(`clippy::linkedlist`), and the code judge (stage 6) judges the choice,
through quality-gate §5.
