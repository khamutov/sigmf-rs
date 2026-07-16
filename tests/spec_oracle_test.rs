//! The specification is the oracle.
//!
//! `tests/spec/sigmf-schema.json` is SigMF's own machine-readable definition of
//! itself. These tests point it at two things: at our fixtures, so the corpus is
//! proven honest before it is used to judge the crate, and at the crate's own
//! output, so "is this spec-conformant?" is an assertion rather than a reading of
//! prose. There is no prose left to read — upstream withdrew `sigmf-spec.md` and
//! now generates its documentation *from* this schema.
//!
//! # Why some tests here are `#[ignore]`d
//!
//! Each ignored test pins a defect that exists right now, and states the change
//! that will clear it. They are written before the fixes, not after, and that
//! ordering is the point: a test written after a fix tests the fix, while a test
//! written before it tests the bug. Un-ignore one and it should fail for exactly
//! the reason its attribute states — if it fails for a different reason, or
//! passes, the defect was not what we thought it was.
//!
//! Fix a defect by deleting its `#[ignore]`, never by editing its assertion. An
//! assertion that has to change to go green was testing the wrong thing.
//!
//! Run them with:
//!
//!     cargo test --test spec_oracle_test -- --ignored

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{json, Value};
use sigmf::sigmf::*;

/// The SigMF version vendored at `tests/spec/sigmf-schema.json`, as it appears in
/// the schema's own `$id`.
///
/// This constant is the whole reason the vendored file needs no header comment:
/// the schema records its own version in-band, and
/// [`vendored_schema_is_the_pinned_version`] asserts the two agree. Refreshing the
/// schema without consciously accepting the new spec version fails the suite
/// instead of sliding in unnoticed. See `tests/spec/README.md`.
const EXPECTED_SPEC_VERSION: &str = "v1.2.6";

/// Every fixture in the corpus.
///
/// All of them must satisfy the schema — the oracle checks our test data before
/// our test data checks the crate. The crate's own inline fixture in `src/test.rs`
/// does *not* satisfy it (its `core:sha512` is `"f4984"`, five characters where
/// the schema demands 128 hex digits), which is why this is not a formality.
const FIXTURES: &[&str] = &[
    "sample.sigmf-meta",
    "minimal.sigmf-meta",
    "global_geolocation.sigmf-meta",
    "capture_geolocation.sigmf-meta",
    "extensions.sigmf-meta",
    "scoped_extension_keys.sigmf-meta",
    "collection.sigmf-meta",
];

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/sigmf_test_files")
        .join(name)
}

fn read_json(path: &Path) -> Value {
    let raw =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn spec_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/spec/sigmf-schema.json"))
    })
}

fn validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        jsonschema::validator_for(spec_schema())
            .expect("the vendored schema must itself be a valid JSON Schema")
    })
}

/// Assert `instance` satisfies the SigMF schema, reporting *every* violation with
/// its JSON path rather than just the first.
fn assert_valid(instance: &Value, what: &str) {
    let errors: Vec<String> = validator()
        .iter_errors(instance)
        .map(|e| format!("  at {}: {e}", e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{what} does not satisfy the SigMF {EXPECTED_SPEC_VERSION} schema:\n{}",
        errors.join("\n")
    );
}

/// Collect every key path in a JSON document, e.g. `global/core:datatype` and
/// `captures/0/core:geolocation/coordinates`.
///
/// Array elements contribute their index to the path of their children but are
/// not themselves keys, so `captures/0` never appears while
/// `captures/0/core:sample_start` does.
fn collect_keys(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}/{key}")
                };
                out.insert(path.clone());
                collect_keys(child, &path, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_keys(child, &format!("{prefix}/{index}"), out);
            }
        }
        _ => {}
    }
}

fn key_paths(value: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_keys(value, "", &mut out);
    out
}

/// Open a fixture and serialize it straight back out, returning the result as JSON.
fn round_trip(name: &str) -> Value {
    let reopened = SigMF::from_file(fixture_path(name))
        .unwrap_or_else(|e| panic!("{name} must open, but did not: {e}"));
    let written = reopened
        .metadata
        .to_str()
        .unwrap_or_else(|e| panic!("{name} must serialize, but did not: {e}"));
    serde_json::from_str(&written)
        .unwrap_or_else(|e| panic!("{name} serialized to something that is not JSON: {e}"))
}

/// A read→write cycle must not lose a single key, in any scope, and must leave a
/// document the specification still accepts.
fn assert_round_trip_preserves_keys(name: &str) {
    let before = key_paths(&read_json(&fixture_path(name)));
    let written = round_trip(name);
    let after = key_paths(&written);

    let lost: Vec<&String> = before.difference(&after).collect();
    assert!(
        lost.is_empty(),
        "{name}: a read→write cycle silently dropped {} key(s), reporting success:\n  {}",
        lost.len(),
        lost.iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_valid(&written, &format!("{name} after a read→write cycle"));
}

/// The vendored schema is the version we think it is.
///
/// Guards the refresh procedure in `tests/spec/README.md`: overwriting the schema
/// with a newer upstream release turns this red, forcing the version bump to be a
/// decision rather than an accident.
#[test]
fn vendored_schema_is_the_pinned_version() {
    let id = spec_schema()["$id"]
        .as_str()
        .expect("the vendored schema must carry an $id recording its version");
    assert!(
        id.contains(EXPECTED_SPEC_VERSION),
        "vendored schema is {id}, but this suite is written against \
         {EXPECTED_SPEC_VERSION}. If the upgrade is deliberate, update \
         EXPECTED_SPEC_VERSION and tests/spec/README.md, then re-run: fixtures \
         that stop validating are the spec telling you what changed."
    );

    assert_eq!(
        spec_schema()["$schema"],
        json!("https://json-schema.org/draft/2020-12/schema"),
        "the validator is built for draft 2020-12"
    );
}

/// Our test data is honest before it is allowed to judge the crate.
#[test]
fn every_fixture_validates_against_the_spec_schema() {
    for name in FIXTURES {
        assert_valid(&read_json(&fixture_path(name)), name);
    }
}

/// The smallest legal recording: the two required globals and nothing else.
#[test]
fn minimal_recording_opens() {
    let sigmf = SigMF::from_file(fixture_path("minimal.sigmf-meta")).expect("minimal must open");
    assert_eq!(sigmf.metadata.captures.len(), 0);
    assert_eq!(sigmf.metadata.annotations.len(), 0);

    // Asserted through the serialized form rather than the typed field, because
    // `datatype` is a String today and is due to become a parsed DataFormat.
    // Testing the observable JSON lets this assertion survive that retype
    // unchanged, which is the whole point: a test the fix has to rewrite proves
    // nothing about the fix.
    let written = round_trip("minimal.sigmf-meta");
    assert_eq!(written["global"]["core:datatype"], json!("cf32_le"));
    assert_eq!(written["global"]["core:version"], json!("1.2.6"));
}

/// A recording declaring `core:extensions` opens, and its declaration survives.
///
/// **This test is green today, and its job is to stay green.** It is the guard
/// against the tempting one-word "fix" to the `core:extensions`/`core:collection`
/// rename collision: correcting the rename *without* also correcting the field's
/// type makes this exact file stop opening, because a spec-shaped extensions array
/// cannot deserialize into an `Option<String>`. Today the mismatched key falls
/// through to the `other` catch-all and is preserved verbatim; the rename alone
/// would trade that for a hard failure. If this test ever goes red, the change that
/// did it is a regression, not a fix.
#[test]
fn declared_extensions_recording_opens_and_survives_a_round_trip() {
    assert_round_trip_preserves_keys("extensions.sigmf-meta");

    let written = round_trip("extensions.sigmf-meta");
    let declared = written["global"]["core:extensions"]
        .as_array()
        .expect("the declared extension list must survive a round-trip");
    assert_eq!(declared.len(), 2);
    assert_eq!(
        written["global"]["antenna:model"],
        json!("Wellbrook ALA1530")
    );
}

#[test]
fn collection_recording_opens_and_survives_a_round_trip() {
    assert_round_trip_preserves_keys("collection.sigmf-meta");
}

#[test]
fn sample_recording_survives_a_round_trip() {
    assert_round_trip_preserves_keys("sample.sigmf-meta");
}

#[test]
fn minimal_recording_survives_a_round_trip() {
    assert_round_trip_preserves_keys("minimal.sigmf-meta");
}

/// A recording with a `core:geolocation` cannot be opened at all.
///
/// `core:geolocation` is typed `Option<String>` where the schema says GeoJSON
/// Point object. Unlike the extensions collision, nothing rescues this: a typed
/// field whose shape mismatches is a hard deserialize error, so the whole file is
/// rejected. `core:geolocation` is one of the most commonly populated optional
/// fields in real recordings, which means anyone who pointed this crate at a
/// real-world file hit this immediately.
#[test]
#[ignore = "known-red: core:geolocation is typed Option<String>; clears when it becomes a typed GeoJSON Point"]
fn global_geolocation_recording_opens() {
    let sigmf = SigMF::from_file(fixture_path("global_geolocation.sigmf-meta"))
        .expect("a recording carrying a global core:geolocation must open");
    assert_eq!(sigmf.metadata.captures.len(), 1);
    assert_round_trip_preserves_keys("global_geolocation.sigmf-meta");
}

/// The schema states the Captures scope is the *preferred* home for
/// `core:geolocation`. The crate has no such field, and `CaptureMetadata` has no
/// catch-all either, so the preferred spelling of the most common optional field
/// is read, discarded, and reported as success.
#[test]
#[ignore = "known-red: CaptureMetadata has no core:geolocation field and no catch-all; clears when it gains either"]
fn capture_scope_geolocation_survives_a_round_trip() {
    assert_round_trip_preserves_keys("capture_geolocation.sigmf-meta");

    let written = round_trip("capture_geolocation.sigmf-meta");
    assert_eq!(
        written["captures"][0]["core:geolocation"]["coordinates"],
        json!([14.5053, -22.9576, 7.0]),
        "GeoJSON is longitude, latitude, altitude — in that order"
    );
}

/// `core:collection` lands in the wrong typed field.
///
/// Two fields carry the same `#[serde(rename = "core:collection")]`: `extensions`
/// and `collection`. serde binds the first, so a file's `core:collection` value
/// arrives in `extensions`, and `collection` is unreachable — permanently `None`.
/// Nothing warns: rustc and clippy are both silent, because serde's derive expands
/// the field-name match into generated code the `unreachable_patterns` lint never
/// sees. A round-trip test passes with this bug present, because the value
/// serializes back out under the same (wrong) name.
///
/// This one is not hygiene. `core:collection` is the field that associates a
/// Recording with a `.sigmf-collection`, and the schema recommends Collections
/// over `core:num_channels` for multi-channel IQ — which is exactly the shape a
/// six-band receiver needs.
#[test]
#[ignore = "known-red: `extensions` and `collection` share a serde rename; clears when `extensions` is renamed to core:extensions AND retyped together -- see the doc comment, the rename alone is a regression"]
fn collection_field_binds_the_core_collection_key() {
    let sigmf = SigMF::from_file(fixture_path("collection.sigmf-meta")).expect("must open");
    assert_eq!(
        sigmf.metadata.global.collection,
        Some("walvisbay-2026-07-16T09-14-22".to_string()),
        "core:collection must arrive in the `collection` field"
    );
}

/// Capture-scope and annotation-scope keys are silently discarded.
///
/// `GlobalMetadata` has a `#[serde(flatten)] other` catch-all. `CaptureMetadata`
/// and `AnnotationMetadata` do not, so every key they do not model is dropped on
/// read and absent on write — while the write reports success.
///
/// This is the one defect on the list that is a data-integrity bug rather than a
/// conformance bug. A reader that cannot see a field is inconvenient. A writer
/// that deletes a field it could not see, and tells you it succeeded, is
/// something else.
#[test]
#[ignore = "known-red: CaptureMetadata and AnnotationMetadata have no #[serde(flatten)] catch-all; clears when they gain one"]
fn scoped_extension_keys_survive_a_round_trip() {
    assert_round_trip_preserves_keys("scoped_extension_keys.sigmf-meta");

    let written = round_trip("scoped_extension_keys.sigmf-meta");
    assert_eq!(
        written["captures"][0]["antenna:azimuth_angle"],
        json!(137.5)
    );
    assert_eq!(
        written["annotations"][0]["capture_details:emitter"],
        json!("coast-station")
    );
}

/// Writing extension data must also declare the extension.
///
/// The schema is explicit that `core:extensions` is how a reader learns it needs
/// to support a namespace before parsing. `set_extension` writes `antenna:model`
/// and never touches the declaration, so the crate emits files that *use* an
/// extension without *declaring* it. The declared-extensions list and the
/// extension accessors are entirely unconnected code — which is plausibly why
/// nobody noticed the field was dead: nothing in the crate ever reads it.
#[test]
#[ignore = "known-red: set_extension writes namespaced keys but never declares the namespace; clears when it maintains core:extensions"]
fn set_extension_declares_the_extension_it_writes() {
    let mut sigmf = SigMF::from_file(fixture_path("minimal.sigmf-meta")).expect("must open");
    sigmf
        .metadata
        .global
        .set_extension(AntennaGlobal {
            model: "Wellbrook ALA1530".to_string(),
            ..Default::default()
        })
        .expect("set_extension must succeed");

    let written: Value = serde_json::from_str(&sigmf.metadata.to_str().expect("serialize"))
        .expect("output must be JSON");

    assert_eq!(
        written["global"]["antenna:model"],
        json!("Wellbrook ALA1530"),
        "the extension data went in"
    );

    let declared = written["global"]["core:extensions"]
        .as_array()
        .expect("writing antenna:* must declare the antenna extension in core:extensions");
    assert!(
        declared.iter().any(|e| e["name"] == json!("antenna")),
        "core:extensions must name the antenna namespace, got {declared:?}"
    );

    assert_valid(&written, "metadata produced by set_extension");
}
