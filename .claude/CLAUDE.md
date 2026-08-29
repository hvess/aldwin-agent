# Mjolnir

## Project Overview

Mjolnir is a Rust TUI coding agent — a discussion-first harness where the developer's understanding is the product, not the agent's throughput. It is not a mobile SDK project. Do not apply mobile SDK, FFI, Android, or iOS framing here.

Workspace: seven Cargo crates under `crates/`. Specs for all seven live in `.claude/spec/`. Read the relevant spec before working on any crate.

## Language & Platform

All code is Rust. Idioms are Rust idioms — do not translate patterns from Kotlin, Swift, or other languages. The relevant references are the Rust Book, std docs, and crate documentation (ratatui, crossterm, reqwest, rmcp, serde).

## Spec Workflow

Specs are in `.claude/spec/` — read before implementing. As of 2026-08-29, four (config, core, llm, cli) are archived under `.claude/spec/archive/` — implemented, tested, and audited with no known gaps. The remaining three (permissions, tools, tui) stay active, each with a dated Progress note on its one or two known gaps. When a spec step is completed, note it; when all steps are done, move the spec to `.claude/spec/archive/`.

## Key Constraints (non-negotiable)

- Default-deny permissions: no tool may act without an explicit grant. No "obviously safe" carve-out.
- Edit is never allowlistable: friction on Edit is structural, not a setting.
- Discussion-first: resting state is conversation. Action only on explicit developer signal.
- No Anthropic wire types past `LlmClient`: audit at the trait boundary, not after.
