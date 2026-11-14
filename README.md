# begone-slop-rs

`cargo bs`: lint rules that reject low-evidence Rust. It's the Rust sibling of
[begone-slop](https://github.com/jliocsar/begone-slop), the oxlint plugin.

Clippy already polices most of what begone-slop polices in TypeScript, so a lot of this package
is a clippy preset. The rest is what clippy and rustfmt can't express:

- a syntax engine, built on `syn`, for blank-line padding, cyclomatic complexity and barrel
  modules, with `--fix`
- ast-grep pattern rules for errors that lose their evidence

## Install

It isn't on crates.io. Install it from git, pinned to a tag or commit:

```sh
cargo install --locked --git https://github.com/jliocsar/begone-slop-rs --tag v0.1.0 cargo-bs
```

To pin it per project with [mise](https://mise.jdx.dev), so every checkout gets the same build:

```sh
mise use 'cargo:https://github.com/jliocsar/begone-slop-rs@tag:v0.1.0'
```

The pattern rules run through the [ast-grep](https://ast-grep.github.io) CLI, so it has to be on
`PATH` (`cargo install ast-grep` or `mise use ast-grep`). Without it, `cargo bs` fails and says
so. `--no-ast-grep` skips the pattern rules.

## Use

```sh
cargo bs              # check the current directory
cargo bs --fix        # fix what can be fixed, report the rest
cargo bs src tests    # check some paths
cargo bs init         # write clippy.toml + rustfmt.toml, print the lints table
```

Inside a git checkout it reads what git tracks plus untracked files that aren't ignored. The exit
code is 0 when clean, 1 on findings, 2 on a usage error. Run `cargo fmt` before `cargo bs --fix`:
formatting moves line breaks, and the padding rules judge the result. rustfmt keeps any blank lines
it finds, so the two never fight.

## The clippy preset

`cargo bs init` writes `clippy.toml` and `rustfmt.toml` when the project has none. It also prints
the `[workspace.lints]` tables from [`preset/lints.toml`](preset/lints.toml) for you to paste. It
never edits your Cargo.toml.

| begone-slop (TypeScript)                    | Rust                                                                 |
| ------------------------------------------- | -------------------------------------------------------------------- |
| `id-length` (2+, except `i`/`j`)            | `min_ident_chars` + `allowed-idents-below-min-chars`                 |
| SCREAMING_SNAKE constants                   | rustc's own `non_upper_case_globals`, which needs no type info       |
| `require-safety-comment-for-type-assertion` | `undocumented_unsafe_blocks`, plus `allow_attributes_without_reason` |
| `no-silent-error-swallow`                   | `let_underscore_must_use`, `map_err_ignore`, `no-discarded-ok`       |
| `no-else-return`                            | `redundant_else` (pedantic)                                          |
| `max-lines-per-function`                    | `too_many_lines` (pedantic)                                          |
| `complexity`                                | `complexity` below, plus `excessive-nesting-threshold = 5`           |
| `no-try-catch`                              | `no-catch-unwind`                                                    |
| `no-unknown-parameters` and friends         | `no-untyped-json`; the type system covers the rest                   |
| `no-reexport-only-modules`                  | `reexport-only-module`, `no-glob-reexport`                           |

Clippy's own `cognitive_complexity` is left off. Clippy's docs say it doesn't measure what it
claims to, so `cargo bs` counts cyclomatic complexity itself.

The Effect rules have no Rust counterpart. `no-switch` doesn't port, since `match` is the idiom.
`no-comments` doesn't port either: rustdoc lives in comments.

## Syntax rules

| Rule                   | What it rejects                                                                       | Fix |
| ---------------------- | ------------------------------------------------------------------------------------- | :-: |
| `padding`              | missing or extra blank lines between statements and items                             | ✅  |
| `assert-padding`       | in tests, an assertion run not fenced off by blank lines, or one with blanks inside   | ✅  |
| `complexity`           | a function with cyclomatic complexity over 15                                         |  —  |
| `reexport-only-module` | a module file that only re-exports (`lib.rs` and `main.rs` are exempt)                |  —  |

`padding` is begone-slop's `padding-line-between-statements` preset, read for Rust:

- a blank line before a `return`
- a blank line around anything that owns a block: `if`, `match`, loops, a multi-line `let` bound to
  one of those, `let ... else`, and a closure with a block body
- a blank line around functions, `impl`s, traits, inline modules and other multi-line items
- a blank line after the imports, with any spacing between them
- one-line `const`/`static` items of the same visibility grouped with no blank line inside the
  group, and a blank line around it
- one-line `let`s may sit together

A comment in the gap counts as introducing what follows it. So a blank line above a comment is
never removed, and a missing one is added above the comment, not between it and its code.

`assert-padding` is `expect-padding`. It applies inside `#[test]` functions, `#[cfg(test)]` modules
and `tests/`. It covers `assert!`, `assert_eq!`, `assert_ne!`, `assert_matches!` and their
`debug_` forms.

`complexity` starts at 1 and adds one for each `if`, loop, `&&`, `||`, `let ... else`, and match
arm past the first, plus one per guard. `?` doesn't count: it's how Rust says "and then", and
counting it would punish the code that handles every error. Closures count toward their function,
and nested `fn`s are measured on their own.

Waive a finding with a comment on its line or the line above. The reason is required:

```rust
// bs-allow(complexity): one arm per key name reads better than any split
fn key_name(key: Key) -> &'static str {
```

## Pattern rules

| Rule                   | What it rejects                                                   |
| ---------------------- | ----------------------------------------------------------------- |
| `no-string-error`      | `Result<T, String>` and `Result<T, &str>`                         |
| `no-box-dyn-error`     | `Box<dyn Error>` outside binaries, build scripts, tests, examples |
| `no-stringified-error` | `.map_err(\|e\| e.to_string())`, `format!` and friends            |
| `no-discarded-ok`      | `result.ok();` as a statement                                     |
| `no-catch-unwind`      | `catch_unwind` used as try/catch                                  |
| `no-glob-reexport`     | `pub use module::*`                                               |
| `no-untyped-json`      | `serde_json::Value`, the `unknown` of Rust                        |

Waive one with ast-grep's own `// ast-grep-ignore: <rule>` comment, and a reason next to it. The
rule files live in [`rules/`](rules), with tests in [`rule-tests/`](rule-tests). `sgconfig.yml`
points the ast-grep CLI and its editor language server at them.

## Developing

This repo checks itself with the tool it builds. In here, `cargo bs` is an alias (in
`.cargo/config.toml`) for the binary built from this tree. A change to a rule is held to that rule
before it ships. The presets are this repo's own `clippy.toml`, `rustfmt.toml` and lints table. A
test fails if `Cargo.toml` drifts from `preset/lints.toml`, and another runs the syntax rules over
`src/` and `tests/`.

```sh
git config core.hooksPath .githooks   # the gate on every commit
scripts/check                         # fmt, clippy, tests, cargo bs, ast-grep tests
```
