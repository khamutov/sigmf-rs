# sigmf — a Rust library for SigMF recordings

SigMF (Signal Metadata Format) is an open specification for recorded radio
signals. A recording is two files sharing a basename: `foo.sigmf-data` holds raw
interleaved samples with no header, `foo.sigmf-meta` holds a JSON object
describing them (sample format, sample rate, centre frequency, timestamp). The
point of the format is that the numbers needed to interpret the bytes travel
*with* the bytes instead of living in a README or someone's memory.

This crate is **published on crates.io under MIT** and is read and used by people
outside this team. That fact drives several conventions below.

## Build and test

Plain cargo; there is no Taskfile here.

    cargo test                                   # unit + integration + doc-tests
    cargo test -- --ignored                      # the known-red tests (see below)
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check

## The specification is the authority, and it is executable

`tests/spec/sigmf-schema.json` is the specification's own machine-readable
definition, vendored verbatim. It is not documentation — it is the test oracle:
fixtures are validated against it before they are used to judge the crate, and
metadata the crate writes is validated against it on the way out. Prefer
"assert against the schema" over "read the spec and argue about intent". There is
no prose spec left to read; upstream withdrew it and now generates its docs *from*
this schema.

Refreshing it to a new spec version is a deliberate act, not a drive-by. See
`tests/spec/README.md`.

Two limits worth knowing. The schema constrains **structure**, not **semantics** —
it will happily accept a field whose meaning you have inverted. And its prose
descriptions are not authoritative against each other: `core:extensions.optional`
is described one way by its parent and the opposite way by its own line. The
oracle raises the floor; it does not remove the need to read.

## Known-red tests

Some tests are `#[ignore]`d because they fail today, each pinning a real defect
and naming the condition that will clear it. They are written *before* the fixes:
a test written after a fix tests the fix, while a test written before it tests the
bug. When you fix something, remove the `#[ignore]` — do not rewrite the
assertion. If an assertion has to change to go green, it was testing the wrong
thing (see the note on typed fields under Tests).

# Code conventions

## Comments & docs

- Comment the *why*, not the *what*. If removing a comment wouldn't confuse the
  reader, delete it. Identifiers and types should carry the *what*.
- Don't decorate source with ASCII banner separators (`// =====`, `// -----`,
  boxed section headers). Section structure belongs in docs, not in `.rs` files.
  In tests, if you reach for a banner to group related tests, that grouping wants
  to be a `mod` with a doc comment.
- Document every public surface: items exported from a module, `pub` structs and
  fields, `pub fn` / `pub async fn`, and any new config field. Also document
  non-obvious internals. Self-explanatory small functions are exempt.
- When a comment cites authority, cite a long-lived artifact a reader can
  actually open. Here that means **the SigMF specification** — quote the schema by
  field name, or point at `tests/spec/sigmf-schema.json`.
  This crate is public; the design docs and execution plans of the private
  monorepo that consumes it are not. Never cite them. A comment reading
  `red until M3 (S-009)` is a dangling pointer for every reader outside this team,
  and goes stale for us the moment the plan is archived. **Inline the decision
  itself** — state the defect and the condition that clears it, in words that mean
  something to someone who has only this repository.

## Tests

- Test contracts, not implementation. A test should fail when the observable
  behavior changes, not when a private helper is renamed.
- Exception: a regression test pinned to a specific bug may legitimately reach
  into internals — say so in the test name or a one-line comment.
- Corollary, and the reason the known-red tests work: assert against a field's
  **serialized JSON** rather than its Rust type whenever that type is expected to
  change. An assertion written against today's `Option<String>` has to be rewritten
  by the change that fixes it, and a test you edited on the way past proves
  nothing about the fix.

## Style

- Follow the Google style guide for the language. Google publishes none for Rust,
  so the project-level substitute is: `rustfmt` defaults (the Rust style guide) for
  layout, and the Rust API Guidelines for naming and public surface. Both are
  enforced by `cargo fmt --check` and `cargo clippy -- -D warnings`.
- Keep the dependency tree small and current, and take dependencies with
  `default-features = false` where the default set buys something the crate does
  not use. Currency is not hygiene here: this crate carried a duplicate-`serde`-
  rename bug in silence for its entire life because its lockfile was pinned to a
  2023 serde whose derive predated the `unreachable_patterns` diagnostic that
  catches it. A frozen lockfile freezes your diagnostics along with it.
