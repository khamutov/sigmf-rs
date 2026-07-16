# Vendored SigMF specification

`sigmf-schema.json` is the SigMF specification's own machine-readable definition,
copied verbatim from upstream. It is the oracle the test suite validates against:
fixtures are checked against it before they are used to check the crate, and
metadata the crate writes is checked against it on the way out.

**Current version: v1.2.6** (JSON Schema draft 2020-12).

The file is byte-identical to upstream and must stay that way — no local edits,
no header comment. That is what makes a refresh a diff instead of a merge. The
version is recorded in-band in the schema's own `$id`, and
`spec_oracle_test.rs::vendored_schema_is_the_pinned_version` asserts it, so
refreshing to a new spec version fails the suite rather than sliding in unnoticed.

## Why vendored rather than fetched

Tests must not need the network, and a spec that changes underneath CI turns an
unrelated PR red. Upstream also has no stability guarantee for `main`. Pinning
means a spec release is something we adopt deliberately.

The `jsonschema` dev-dependency is built with `default-features = false` for the
same reason: the default feature set includes `resolve-http`, which would let the
validator fetch a remote `$ref` mid-test. This schema has no `$ref`s, so the
resolver is dead weight — and without it, a future schema that grows one fails
loudly instead of quietly reaching the network.

## Refreshing to a new spec version

Deliberately, and never as a drive-by:

    gh api repos/sigmf/SigMF/contents/sigmf-schema.json --jq '.content' | base64 -d > tests/spec/sigmf-schema.json

Then read the new `$id`, update `EXPECTED_SPEC_VERSION` in
`tests/spec_oracle_test.rs` and the version above to match, and re-run the suite.
Fixtures that stop validating are the point of the exercise: they are the spec
telling you what changed.

Upstream: <https://github.com/sigmf/SigMF/blob/main/sigmf-schema.json>

The prose specification (`sigmf-spec.md`) that this crate's README used to link to
no longer exists; upstream generates prose *from* this schema via its
`docs-generator.py`. The schema is now the authority.
