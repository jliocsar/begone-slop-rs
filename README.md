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
cargo bs -- -odd.rs   # everything after `--` is a path
cargo bs init [DIR]   # write clippy.toml + rustfmt.toml into DIR (default .), print the lints table
cargo bs --version    # or -V
```

`init` takes only `-h`/`--help` and one directory, so `cargo bs init --help` prints its usage
and writes nothing.

Inside a git checkout it reads exactly what git lists: tracked files plus untracked files that
aren't ignored. The pattern rules get that same list, file by file, with ast-grep's own ignore
files and any project `sgconfig.yml` switched off, so neither can hide a file. A path with no
such files (say, a gitignored directory) is reported, and so is a file `syn` can't parse: no
rule could check it, so it doesn't pass. The exit code is 0 when clean, 1 on findings, 2 on a
usage error, a `bs.toml` entry that names nothing, or when ast-grep is missing. `--fix` applies
its fixes pass after pass until none is left; if they still haven't settled after 8 passes (two
fixes undoing each other, which is a bug in `cargo bs`), it leaves that file as it was and says
so. Run `cargo fmt` before `cargo bs --fix`:
formatting moves line breaks, and the padding rules judge the result. rustfmt keeps any blank lines
it finds, so the two never fight.

## Keeping the gate shut

The quickest way past a lint gate is to weaken the gate, so `cargo bs` checks that too.

- `preset-drift`: every lint the preset turns on ends up at the preset's level or stricter, read
  the way Cargo and rustc resolve the table: a group sets every lint in it, and the entry with
  the highest `priority` wins. So `too_many_lines = "allow"` (a pedantic lint) and
  `restriction = { level = "allow", priority = 1 }` both fail, and `unwrap_used = "deny"`
  passes. Where two entries tie, the looser one counts. `warnings = "allow"` and an allow of a
  lint `cargo bs` doesn't know fail too. Every workspace member inherits the workspace lints
  (`[lints] workspace = true`). Every `clippy.toml` (the one clippy finds at or above the root,
  and any below it) may only tighten: thresholds move the strict way, exemption lists
  (`allowed-*`) shrink, ban lists (`disallowed-*`) grow, `allow-*` switches turn off. A key it
  doesn't set counts as clippy's default, so `too-many-lines-threshold = 1000` fails even though
  the preset never writes it down; `msrv` and unknown keys fail. A manifest or config that can't
  be read or parsed is a finding.
- `cargo-config-drift`: `.cargo/config.toml` and `.cargo/config`, in the root and every directory
  above it (Cargo reads them all), may not turn lints down: no `-A`/`--allow`/`--cap-lints` in
  `rustflags` (it beats the lints table), no `-W` on a lint the preset denies, no `[env]`
  `CLIPPY_CONF_DIR`/`RUSTFLAGS`/`RUSTC_WRAPPER`…, no `build.rustc-wrapper`, and no alias named
  `clippy`, `fmt` or `bs`, except the in-tree `bs = "run --bin cargo-bs -- bs"` bootstrap. The
  same flags or `CLIPPY_CONF_DIR` in `cargo bs`'s own environment fail the run.

Both are checked from the project's root: the workspace that includes the crate `cargo bs` runs
in, so running it from a member crate checks the same thing. There's no waiver for either.
- `no-forbidden-expects`: outside tests, the lints that guard the gate can't be waived at all, not
  even with a reason: `unwrap_used`, `expect_used`, `panic`, `todo`, `unreachable`, `exit`,
  `dead_code`, `unsafe_code`, `allow_attributes` and a few more; every lint the preset turns on
  by name; every group (`clippy::restriction`, `clippy::style`, `warnings`, `nonstandard_style`…,
  since a group waives its members); the whole `unused*` family; and `renamed_and_removed_lints`
  and `unknown_lints`. An old name (`clippy::option_unwrap_used`) counts as its current one.
- `no-crate-level-allow`: no `#![allow]`/`#![expect]` for a whole file or module, none on a `mod`
  (`#[allow(..)] mod big;` waives a whole file), none behind `cfg_attr` at any depth, and none
  written inside a macro (`macro_rules!` bodies and invocations), where it lands on code nobody
  sees.
- `no-hidden-code`: `#[path = "core.inc"] mod core;` and `include!("more.in")` compile code that
  no rule reads, and so does either one pointing at a `.rs` file git doesn't list. Only Cargo's
  generated-code idiom, `include!(concat!(env!("OUT_DIR"), ...))`, passes.
- `waiver-needs-reason`: a waiver names what it waives and says why, in a sentence (3 words or
  more). A `bs-allow` only counts in a plain `//` comment, not in a string, a doc comment or a
  block comment, and naming a gate rule or a pattern rule is a finding. ast-grep
  honours any comment that opens with `ast-grep-ignore`, so the only one that passes is a plain
  `//` comment on its own line, right above the code it waives, naming pattern rules and a
  reason: `// ast-grep-ignore: no-catch-unwind -- the FFI boundary must not unwind`. A bare one
  (it waives every rule), one in a `/* */` or doc comment, one after code on its line, and one
  above a blank line (on line 1, that waives the whole file) all fail. This is a syntax rule, not
  a pattern rule, so ast-grep can't waive it.
- `unknown-waiver`: a `bs-allow` naming a rule that doesn't exist (`bs-allow(complexty)`) waives
  nothing, so it's a finding rather than a silent no-op.
- `unused-waiver`: a `bs-allow` that takes no finding off the report, because the code it
  excused changed or it sits a line too far from it, is a finding: delete it. Like rustc's
  `unfulfilled_lint_expectations`, it keeps old waivers from hiding the next finding there. A
  waiver that's already a `waiver-needs-reason` finding isn't reported again.

None of these rules can be waived.

## Boundaries: `bs.toml`

Two rules need to know the project's architecture, so they read an optional `bs.toml` at the
root. Without one, they're off.

```toml
[io]
# The shell: the only files allowed to touch the outside world. Everything else is pure.
allowed = ["src/main.rs", "src/shell"]
# Effects of your own, on top of the built-in list.
effects = ["sqlx"]

[[layers]]
crate = "crates/client"
# Top first. A module may import from its own layer and the ones below, never above.
order = ["crate::conn", "crate::proto"]
```

- `io-boundary`: functional core, imperative shell. Outside `allowed`, nothing may reach the file
  system, network, processes, environment, stdio (`print!` and friends included), clock,
  randomness or signals. It's checked where an effect is imported, and where a full path names it
  in code, macro arguments (`format!`, `dbg!`) included. Values that only live under those modules
  (`SocketAddr`, `IpAddr`, `ExitCode`, `std::env::consts`, `fs::Metadata`) aren't effects. `Path`'s
  own file-system methods are: `.exists()`, `.try_exists()`, `.canonicalize()`, `.metadata()`,
  `.symlink_metadata()`, `.read_dir()`, `.read_link()`, `.is_file()`, `.is_dir()` and
  `.is_symlink()`. The receiver's type isn't known, so any call by those names counts, except
  `.is_file()`/`.is_dir()`/`.is_symlink()` on what `metadata()` or `file_type()` just returned or
  on a binding named for it (`kind`, `file_type`, `meta`). Test code is exempt.
- `layers`: each `use`, and each path written from `crate`/`self`/`super`, points down the
  declared order. Between crates, Cargo's dependency graph already does this.

A typo in `bs.toml` would switch a boundary off without a word, so `cargo bs` checks every entry
when it loads the file and exits 2 naming the first bad one: each `allowed` path has to exist,
each `effects` entry has to be a path (`sqlx`, `tokio::fs`), each `crate` has to be a directory
with a `Cargo.toml`, and each `order` entry has to be a `crate::` path to a module of that crate
(a `src/` file or directory, or an inline `mod` in its parent's file).

This repo's own [`bs.toml`](bs.toml) keeps `src/rules/` pure and layered below `check`.

## The clippy preset

`cargo bs init` writes `clippy.toml` and `rustfmt.toml` when the project has none. It also prints
the `[workspace.lints]` tables from [`preset/lints.toml`](preset/lints.toml) for you to paste. It
never edits your Cargo.toml.

| begone-slop (TypeScript)                    | Rust                                                                 |
| ------------------------------------------- | -------------------------------------------------------------------- |
| `id-length` (2+, except `i`/`j`)            | `min_ident_chars` + `allowed-idents-below-min-chars`                 |
| SCREAMING_SNAKE constants                   | rustc's own `non_upper_case_globals`, which needs no type info       |
| `require-safety-comment-for-type-assertion` | `undocumented_unsafe_blocks`, plus `allow_attributes_without_reason` |
| `no-silent-error-swallow`                   | `let_underscore_must_use`, `map_err_ignore`, `unused_result_ok`      |
| `no-else-return`                            | `redundant_else` (pedantic)                                          |
| `max-lines-per-function`                    | `too_many_lines` (pedantic)                                          |
| `complexity`                                | `complexity` below, plus `excessive-nesting-threshold = 5`           |
| `no-try-catch`                              | `no-catch-unwind`                                                    |
| `no-unknown-parameters` and friends         | `no-untyped-json`; the type system covers the rest                   |
| `no-reexport-only-modules`                  | `reexport-only-module`, `no-glob-reexport`                           |
| `statement-order`                           | `arbitrary_source_item_ordering`, grouped in `clippy.toml`           |
| panics in fallible code, dead `pub`         | `panic_in_result_fn`, `unwrap_in_result`, `unreachable_pub`          |
| test assertions that hide the error         | `assertions_on_result_states`                                        |
| one module layout                           | `mod_module_files`: `foo.rs` next to `foo/`, never `foo/mod.rs`      |
| panics the preset's bans don't name         | `unreachable`, `exit` (outside `main`), and `no-panic-bypass` below  |
| test-only branches in shipped code          | `cfg_not_test`, and `no-cfg-test-in-prod` below                      |

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
| `use-groups`           | imports not grouped std → other crates → this crate, one blank line between groups    | ✅  |
| `no-abbreviations`     | a declared name with an abbreviation that needs decoding (`ctx`, `cfg`, `idx`, `msg`…) |  —  |
| `test-name`            | a test named `test_*`, `*_test`, `test2`, `it_works`, `works`, `basic` or `smoke`     |  —  |
| `no-sleep-in-tests`    | `sleep` in test code (`thread::sleep`, `tokio::time::sleep`, …)                       |  —  |
| `test-must-assert`     | a `#[test]` with no assertion, no `#[should_panic]`, and no call to a helper that asserts; `assert!(true)` and other asserts of constants don't count, `.assert()` and `insta::assert_*!` do |  —  |
| `no-mutable-accumulator` | `let mut v = Vec::new(); for x in xs { v.push(…) }`: use `.map().collect()`, `.sum()`, `.fold()` |  —  |
| `prefer-match`         | an `if`/`else if` chain of 3+ tests of one subject against fixed values; against strings outside a parse, it says to parse into an enum (as `no-stringly-match` does) |  —  |
| `no-boolean-trap`      | a `bool` parameter; take a two-variant enum that names both cases                     |  —  |
| `no-stringly-fields`   | a `kind`/`status`/`state`/`mode`/`level`/`role`… field or parameter typed as a string (parse fns exempt) |  —  |
| `no-wildcard-on-local-enum` | `_ =>` or a catch-all binding on an enum this file defines (`Enum::V`, `self::Enum::V`, `Self::V`; not `other::Enum::V`), outside tests |  —  |
| `no-blocking-in-async` | `std::fs`, `std::net`, `std::thread::sleep`, `std::process::Command`, blocking HTTP inside `async` (macro arguments, trait defaults and in-place closures like `map` included; a closure handed to `spawn_blocking`/`spawn`/`block_in_place` isn't) |  —  |
| `no-numbered-names`    | `item2` next to `item`, or `point1` next to `point2`; not `x0`/`x1`, bit widths (`hash64`) or terms like `utf8`, `ipv6`, `sha256`, `base64` |  —  |
| `no-type-in-name`      | `user_vec`, `name_str`, `lookup_hashmap`: the type already says it                    |  —  |
| `no-junk-drawer-modules` | modules named `utils`, `helpers`, `common`, `misc`                                  |  —  |
| `module-doc`           | a file without a `//!` comment saying what it's for (`build.rs` exempt)               |  —  |
| `no-commented-out-code` | a run of `//` comments, each on its own line, that parses as Rust statements (comments are lexed, so text in a string literal is never one) |  —  |
| `newtype-ids`          | an `id`/`*_id`/`*_ids` field or parameter typed `String`, `&str`, an integer or `Uuid` |  —  |
| `no-silent-err-arm`    | `Err(_) => {}` (or `continue`, `break`, `None`, `()`, `return`, `Default::default()`), and `if let Err(_) = .. {}`, outside tests |  —  |
| `no-filter-map-ok`     | `.filter_map(Result::ok)`, `.filter_map(\|r\| r.ok())`, `.flatten()` over `read_dir`/`lines()`: every failed item vanishes |  —  |
| `no-discard-assign`    | `_ = fallible();`, which no rustc or clippy lint sees, and `let _ = name;` (an unused-binding silencer) |  —  |
| `no-panic-bypass`      | `assert!`, `assert_eq!`, `assert_ne!` in shipped code (not tests, examples, benches or build scripts; `const` assertions and `debug_assert!` pass) |  —  |
| `no-anyhow-in-lib`     | `anyhow`, `eyre` or `color_eyre` in library code; binaries, build scripts, tests, examples and benches may use them |  —  |
| `test-conditional-assert` | an assertion in a test that sits under an `if let` with no `else`, or in a `match` next to `_ => {}`: it never runs when the case fails |  —  |
| `test-early-return`    | a `return` (or `return Ok(())`) inside a branch of a `#[test]`: a skip that reports a pass; use `#[ignore = "why"]` |  —  |
| `no-cfg-test-in-prod`  | in shipped code, `cfg!(test)`, `#[cfg(test)]` inside a function body, and `*_for_test(s)`/`*_for_testing`/`test_only_*` functions outside `#[cfg(test)]` |  —  |

Test code, which several rules exempt or only check, is a `#[test]` function, a `#[cfg(test)]`
module (`cfg(all(test, ...))` too), a file in a `tests/` directory directly under a crate's root,
and a file reached through `#[cfg(test)] mod x;` (with or without `#[path]`). The last one comes
from walking each crate's module tree from its targets before the rules run, which also tells
a binary's modules from a library's.

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
arm past the first, plus one per guard. A lookup `match`, where every arm is a plain value (a
literal, a path, or an uppercase constructor like `Some(Kind::Open)` applied to values) and none
has a guard, counts once: it's a table, not a decision tree. An arm that calls a function
(`Op::Open(arg) => open(arg)`) is a dispatch, so every arm counts. `?` doesn't count: it's how Rust says "and then", and
counting it would punish the code that handles every error. Closures count toward their function,
and nested `fn`s are measured on their own.

`use-groups` is rustfmt's unstable `group_imports = "StdExternalCrate"`, on stable. "This crate"
is `crate`, `self`, `super`, and a module or enum declared in the same file (`mod hooks;` makes
`pub use hooks::run;` local); `::std` is std. A blank line may set a group's `pub use` re-exports
apart from its imports. A run of
imports that's out of order is rewritten in place, and rustfmt then sorts inside each group. A run
with a comment inside is only reported, since the comment could belong to either neighbour.

`no-abbreviations` checks only the names your code declares: bindings, parameters, functions,
fields, types, variants and constants. Names used from other crates, and methods in a trait impl,
are left alone. Words the language itself uses, like `str`, `len`, `args` and `fmt`, aren't on the
list. Neither are domain words like `expr` or `stmt`.

`no-boolean-trap`, `newtype-ids` and `no-stringly-fields` read a type alias in the same file as
what it names: `type Flag = bool;` doesn't hide a `bool` parameter.

Waive a finding with a plain `//` comment on its line or the line above. The reason is required,
and it's a sentence of 3 words or more (`fine` doesn't count). The gate rules above can't be
waived:

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
| `no-catch-unwind`      | `catch_unwind` used as try/catch                                  |
| `no-glob-reexport`     | `pub use module::*`                                               |
| `no-untyped-json`      | `serde_json::Value`, the `unknown` of Rust, outside tests         |
| `no-stringly-match`    | string-literal match arms outside a parse (`FromStr`, `TryFrom`, `parse*`, `decode*`, `from_*`) |
| `no-swallowed-parse`   | `.parse().unwrap_or(…)` and friends: bad input becomes a default  |
| `todo-needs-issue`     | `TODO`/`FIXME`/`XXX`/`HACK` without `(ABC-123)`, `(#12)` or a URL |
| `no-global-state`      | a `static mut`, or a `static` holding a `Mutex`, `RefCell`, `Cell` or atomic |
| `no-detached-tasks`    | `tokio::spawn(..)` / `thread::spawn(..)` with the handle dropped |
| `error-message-style`  | `#[error("Capitalised.")]`: lowercase, no trailing period (C-GOOD-ERR) |
| `no-rc-refcell`        | `Rc<RefCell<_>>` and `Rc<Cell<_>>`: give the state one owner      |

A bare `#[ignore]` is clippy's `ignore_without_reason`, which the preset turns on with the
pedantic group. Waive a pattern rule with ast-grep's own comment, on its own line above the code:
`// ast-grep-ignore: <rule> -- <reason>` (`waiver-needs-reason` checks its form). The
rule files live in [`rules/`](rules), with tests in [`rule-tests/`](rule-tests). `sgconfig.yml`
points the ast-grep CLI and its editor language server at them.

## Developing

This repo checks itself with the tool it builds. In here, `cargo bs` is an alias (in
`.cargo/config.toml`) for the binary built from this tree. A change to a rule is held to that rule
before it ships. The presets are this repo's own `clippy.toml`, `rustfmt.toml` and lints table. A
test fails if `Cargo.toml` drifts from `preset/lints.toml`, and another runs the syntax rules over
`src/` and `tests/`.

A syntax rule is declared once, in the `rules!` table in `src/rules.rs` (its name, whether it
guards the gate, whether `--fix` fixes it), and its check is one more entry in `CHECKS`. A
pattern rule is one more id in `ast_grep::RULES`. Tests fail when this README's rule tables and
lists don't match them.

Rule behaviour is pinned by fixtures: `tests/fixtures/<rule>/<case>.rs.fixture` is a Rust file
whose flagged lines end in `//~ <rule>` (a first line `//@ path: src/x.rs` says where it pretends
to live, and a `bs.toml` beside it is its config). `tests/fixtures.rs` runs every one: the rules
must report exactly the marked lines, and `--fix` must settle, so fixing the fixed text again
changes nothing. The `.rs.fixture` extension keeps the gate, which lints every `.rs` file git
lists, off the slop they hold on purpose. To turn a repro into a regression test, drop it in and
mark its lines.

```sh
git config core.hooksPath .githooks   # the gate on every commit
scripts/check                         # fmt, clippy, tests, cargo bs, ast-grep tests
scripts/lint-table                    # after a toolchain bump: refresh the known lints and groups
```

`preset-drift` resolves groups with `src/lints/known.rs`, which `scripts/lint-table` generates
from `clippy-driver -W help`.
