---
name: rust
description: Idiomatic Rust patterns, code style, project philosophy and the quality checks (cargo fmt, cargo clippy, cargo test) to run after any change to Rust code. Use when writing, editing or reviewing Rust in crates/.
---

# Rust

## Idiomatic Rust Patterns

### Use Concise Control Flow

Prefer `?` operator over explicit `match` statements for error propagation:

```rust
// Good
let result = operation()?;

// Avoid
match operation() {
    Ok(val) => val,
    Err(e) => return Err(MyError::from(e)),
}
```

Use `if let` instead of `match` when handling single patterns:

```rust
// Good
if let Some(value) = option {
    do_something(value);
}

// Avoid
match option {
    Some(value) => do_something(value),
    None => {}
}
```

Use `while let` for loop patterns:

```rust
// Good
while let Some(item) = iterator.next() {
    process(item);
}

// Avoid
loop {
    match iterator.next() {
        Some(item) => process(item),
        None => break,
    }
}
```

### Iterator Patterns

Prefer iterator chains over manual loops:

```rust
// Good
let results: Vec<_> = items
    .iter()
    .filter(|item| item.is_valid())
    .map(|item| item.process())
    .collect();

// Avoid
let mut results = Vec::new();
for item in &items {
    if item.is_valid() {
        results.push(item.process());
    }
}
```

Use functional combinators (`filter_map`, `flatten`, `fold`, etc.) instead of
intermediate collections.

### Error Handling

Use `?` operator over explicit error conversions when error types can be
automatically converted via `From` trait or `#[from]` attribute:

```rust
// Good: Use ? when LspError has #[from] std::io::Error
write_message(&mut *stdin, message).await?;

// Avoid: Explicit map_err when automatic conversion works
write_message(&mut *stdin, message).await.map_err(LspError::Io)?;
```

Use `map_err` only when you need custom error transformation that cannot be
handled by the `From` trait:

```rust
// Good: Custom error context that can't be expressed via From
let mut child = cmd.spawn().map_err(|source| LspError::Spawn { command: command.to_string(), source })?;
```

- Prefer `anyhow` or `thiserror` for application-level error handling
- Use `Result` extensions like `ok_or`, `and_then`, `or_else` for complex flows

### Other Idiomatic Patterns

- Prefer destructuring assignments where appropriate
- Use `..` spread operator in struct updates
- Leverage `From`/`Into` traits for conversions
- Use `AsRef`/`AsMut` bounds for flexible parameter types

### Import Grouping

Group imports from same crate using curly braces:

```rust
// Good: Grouped imports
use aldwin_review::{scene, stages, tokens, Baseline, Compositor};
use aldwin_review::capture::{capture, measure_cell};

// Avoid: Multiple separate imports
use aldwin_review::scene;
use aldwin_review::stages;
use aldwin_review::tokens;
use aldwin_review::Baseline;
use aldwin_review::Compositor;
use aldwin_review::capture::capture;
use aldwin_review::capture::measure_cell;
```

Use qualified paths sparingly — prefer imports over fully-qualified names in
function signatures:

```rust
// Good: Using imported types
pub async fn run() -> Result<(), StartupError> {
    // ...
}

// Avoid: Fully-qualified in signatures (verbose)
pub async fn run() -> Result<(), crate::error::StartupError> {
    // ...
}
```

The same holds in test bodies: a test module imports what its tests use
(`use super::grid::{Ctx, ..}`) rather than spelling `super::grid::Ctx`
inline, and a test does not re-import what its module already brings in.

## Project Philosophy

### Avoid Overengineering

- Implement exactly what's required by task specifications and acceptance criteria
- Cover specified edge cases but avoid adding extra functionality unless explicitly requested
- Focus on essential features — resist the urge to add bells and whistles
- Keep changes minimal and targeted — only modify what's necessary to fulfill requirements
- Refactor only when essential or explicitly requested

## Commands

### Build

```sh
cargo build
cargo build --release
```

### Test

```sh
cargo test --workspace
cargo test -p aldwin-tui                               # one crate
cargo test -p aldwin-tui --test render_snapshot        # the rendered frames
UPDATE_SNAPSHOTS=1 cargo test -p aldwin-tui --test render_snapshot   # only when the frames are meant to change; read the diff first
```

### Run

`aldwin` takes no arguments: every runtime setting lives in `.aldwin/`
(project) and `~/.aldwin/` (global), not flags.

```sh
cargo run -p aldwin-cli
cargo run -p aldwin-cli -- --version
```

### Check

```sh
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

### Design tokens

`crates/tui/src/tokens.rs` is generated — never edit it by hand:

```sh
cargo run -p aldwin-review -- tokens --write
```

## Code Style

### Rust Standards

- Follow standard Rust naming conventions (`snake_case` for functions/variables, `CamelCase` for types)
- Use `thiserror` for custom error types
- Prefer `?` operator over `match` for error propagation
- Use `async`/`await` for I/O operations
- Derive common traits: `Debug`, `Clone`, `PartialEq` where applicable

### Documentation

- Use `///` for public API documentation
- Include examples in the doc comments of public functions a change adds.
  A doc written for an existing item — to satisfy `missing_docs`, or to add
  an `# Errors` section — does not need one (the developer's scoping,
  2026-09-27)
- Document panics and errors

### Testing

- Write unit tests in `#[cfg(test)]` modules
- Use `tempfile` for filesystem tests
- Use `assert_cmd` for CLI integration tests
- A bug fix lands with the test that would have caught it (quality-gate §8)

## Quality Assurance

Upon completion of each task that involves Rust code, AI agents MUST run the
following commands to ensure code quality and consistency.

### Required Checks

```sh
# Format all Rust code
cargo fmt

# Run linter and static analysis — warnings are errors
cargo clippy --workspace --all-targets -- -D warnings

# Run the tests
cargo test --workspace
```

Before committing, run `/review`: an agent's commit is refused without a
passing one. Its stage 2 enforces this skill's checkable rules as workspace
lints, and its stage 7 judges the rest of this skill against the diff.

### Why These Checks Matter

- **`cargo fmt`**: Ensures consistent code formatting across the entire codebase, following Rust's standard style guide
- **`cargo clippy`**: Catches common mistakes, idiomatic issues, and provides suggestions for better Rust code
- **`cargo test`**: Catches regressions, including snapshot changes to the rendered frames

### When to Run

- After completing any implementation task
- Before marking a task as done
- Before creating a pull request or committing changes
- After any code modification in the `crates/` directory

### Handling Errors

If `cargo fmt`, `cargo clippy` or `cargo test` report issues:

1. Fix all warnings and errors reported by clippy
2. Re-run formatting if needed
3. Verify the fixes don't break existing tests
4. Re-run the commands to confirm compliance
