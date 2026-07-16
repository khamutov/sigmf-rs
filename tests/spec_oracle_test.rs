//! The specification is the oracle.
//!
//! `tests/spec/sigmf-schema.json` is SigMF's own machine-readable definition of
//! itself. These tests point it at two things: at our fixtures, so the corpus is
//! proven honest before it is used to judge the crate, and at the crate's own
//! output, so "is this spec-conformant?" is an assertion rather than a reading of
//! prose. There is no prose left to read — upstream withdrew `sigmf-spec.md` and
//! now generates its documentation *from* this schema.
//!
//! # Known-red tests
//!
//! A defect this crate has not yet fixed is pinned here by an `#[ignore]`d test
//! naming the change that will clear it. Such a test is written *before* the fix,
//! not after, and that ordering is the point: a test written after a fix tests the
//! fix, while a test written before it tests the bug. Un-ignore one and it should
//! fail for exactly the reason its attribute states — if it fails for a different
//! reason, or passes, the defect was not what we thought it was.
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
    "geolocation_foreign_members.sigmf-meta",
    "realistic_recording.sigmf-meta",
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

/// Compare two JSON numbers as numbers rather than as text.
///
/// A `core:frequency` of `16804500` is read into an `f64` and written back as
/// `16804500.0`. JSON has a single number type and the schema types the field
/// `number`, so both spellings carry the same value and the schema accepts each;
/// a textual comparison would report a difference that does not exist. Comparing
/// through `as_f64` alone has the opposite failure — it silently equates distinct
/// integers past 2^53 — so integers are compared exactly and the float path is
/// taken only when one side really is a float.
fn numbers_equal(a: &serde_json::Number, b: &serde_json::Number) -> bool {
    if let (Some(x), Some(y)) = (a.as_u64(), b.as_u64()) {
        return x == y;
    }
    if let (Some(x), Some(y)) = (a.as_i64(), b.as_i64()) {
        return x == y;
    }
    a.as_f64() == b.as_f64()
}

/// Record every path at which `before` and `after` carry different values.
///
/// Keys present on one side only are deliberately ignored here; the key-set
/// assertions report those, and with a better message.
fn diff_values(before: &Value, after: &Value, path: &str, out: &mut Vec<String>) {
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, child) in a {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}/{key}")
                };
                if let Some(counterpart) = b.get(key) {
                    diff_values(child, counterpart, &child_path, out);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                out.push(format!(
                    "{path}: array of {} became an array of {}",
                    a.len(),
                    b.len()
                ));
                return;
            }
            for (index, (x, y)) in a.iter().zip(b).enumerate() {
                diff_values(x, y, &format!("{path}/{index}"), out);
            }
        }
        (Value::Number(a), Value::Number(b)) => {
            if !numbers_equal(a, b) {
                out.push(format!("{path}: {a} became {b}"));
            }
        }
        _ => {
            if before != after {
                out.push(format!("{path}: {before} became {after}"));
            }
        }
    }
}

/// A read→write cycle must change nothing, and must leave a document the
/// specification still accepts.
///
/// "Nothing" is three separate claims, and the crate has historically failed the
/// first while looking healthy: no key is lost, no key is invented, and every
/// value that survives is the value that went in. Checking only the first is what
/// let a writer delete a field it could not model and report success.
fn assert_round_trip_is_lossless(name: &str) {
    let before = read_json(&fixture_path(name));
    let written = round_trip(name);

    let before_keys = key_paths(&before);
    let after_keys = key_paths(&written);

    let lost: Vec<&String> = before_keys.difference(&after_keys).collect();
    assert!(
        lost.is_empty(),
        "{name}: a read→write cycle silently dropped {} key(s), reporting success:\n  {}",
        lost.len(),
        lost.iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    let invented: Vec<&String> = after_keys.difference(&before_keys).collect();
    assert!(
        invented.is_empty(),
        "{name}: a read→write cycle invented {} key(s) that the file never carried:\n  {}",
        invented.len(),
        invented
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    let mut changed = Vec::new();
    diff_values(&before, &written, "", &mut changed);
    assert!(
        changed.is_empty(),
        "{name}: a read→write cycle preserved every key but altered {} value(s):\n  {}",
        changed.len(),
        changed.join("\n  ")
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

/// The version the crate writes into `core:version` is the version it is tested
/// against.
///
/// [`vendored_schema_is_the_pinned_version`] catches a schema refresh that nobody
/// acknowledged. This catches the other half: acknowledging it in the test suite
/// but forgetting the crate, leaving it to stamp recordings with a spec version it
/// no longer implements.
#[test]
fn the_version_the_crate_writes_is_the_version_it_is_tested_against() {
    assert_eq!(
        format!("v{SIGMF_VERSION}"),
        EXPECTED_SPEC_VERSION,
        "SIGMF_VERSION is what the crate stamps into core:version; it must track \
         the vendored schema"
    );
}

/// A recording built through the public constructor satisfies the specification.
///
/// The constructor replaced a `Default` impl that produced an empty `core:datatype`
/// and an empty `core:version` — both required by the schema, so every recording
/// built from it and then filled in was born invalid and stayed that way unless the
/// caller happened to overwrite both. `GlobalMetadata::new` cannot do that: it takes
/// a parsed datatype and supplies the version itself.
#[test]
fn a_recording_built_through_the_constructor_validates() {
    let metadata = Metadata {
        global: GlobalMetadata::new("cf32_le".parse().expect("cf32_le is a valid datatype")),
        captures: vec![],
        annotations: vec![],
    };

    let written: Value =
        serde_json::from_str(&metadata.to_str().expect("serialize")).expect("output must be JSON");

    assert_valid(&written, "a recording built through GlobalMetadata::new");
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
/// **This test's job is to stay green**, and it is the reason the fix to the
/// `core:extensions`/`core:collection` rename collision had to correct the field's
/// *type* in the same change as its *name*. Correcting the rename alone would make
/// this exact file stop opening, because a spec-shaped extensions array cannot
/// deserialize into the `Option<String>` the field then had — so the one-word fix
/// turns a red test green and this green test red, and only running both tells you
/// which happened. If this test ever goes red, the change that did it is a
/// regression, not a fix.
#[test]
fn declared_extensions_recording_opens_and_survives_a_round_trip() {
    assert_round_trip_is_lossless("extensions.sigmf-meta");

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
    assert_round_trip_is_lossless("collection.sigmf-meta");
}

#[test]
fn sample_recording_survives_a_round_trip() {
    assert_round_trip_is_lossless("sample.sigmf-meta");
}

#[test]
fn minimal_recording_survives_a_round_trip() {
    assert_round_trip_is_lossless("minimal.sigmf-meta");
}

/// A recording carrying a `core:geolocation` opens.
///
/// It could not until the field was typed. `core:geolocation` was an
/// `Option<String>` where the schema says GeoJSON Point object, and unlike the
/// extensions collision nothing rescued it: a typed field whose shape mismatches
/// is a hard deserialize error, so the whole file was rejected. This is one of the
/// most commonly populated optional fields in real recordings, so anyone who
/// pointed the crate at a real-world file hit it immediately.
#[test]
fn global_geolocation_recording_opens() {
    let sigmf = SigMF::from_file(fixture_path("global_geolocation.sigmf-meta"))
        .expect("a recording carrying a global core:geolocation must open");
    assert_eq!(sigmf.metadata.captures.len(), 1);
    assert_round_trip_is_lossless("global_geolocation.sigmf-meta");
}

/// The Captures scope is the schema's *preferred* home for `core:geolocation`, and
/// a position written there survives intact.
///
/// The crate once had no such field and no capture-scope catch-all behind it
/// either, so the preferred spelling of the most common optional field was read,
/// discarded, and reported as success.
#[test]
fn capture_scope_geolocation_survives_a_round_trip() {
    assert_round_trip_is_lossless("capture_geolocation.sigmf-meta");

    let written = round_trip("capture_geolocation.sigmf-meta");
    assert_eq!(
        written["captures"][0]["core:geolocation"]["coordinates"],
        json!([14.5053, -22.9576, 7.0]),
        "GeoJSON is longitude, latitude, altitude — in that order"
    );
}

/// A recording using every field this milestone typed, at once.
///
/// The three fixes landed here — a Global geolocation, a Captures geolocation, and
/// a declared `core:extensions` — each have their own fixture. This one puts them
/// in a single file, in the shape a real receiver writes, because a Recording that
/// exercises one typed field at a time is not evidence about a Recording that
/// exercises all of them: `core:geolocation` and `core:extensions` are typed fields
/// sitting beside a `#[serde(flatten)]` catch-all, and serde resolves that
/// combination by buffering, which is exactly where a field can go missing without
/// anyone writing a line of wrong code.
#[test]
fn a_realistic_recording_opens_with_every_typed_field_populated() {
    let sigmf = SigMF::from_file(fixture_path("realistic_recording.sigmf-meta"))
        .expect("a recording using all of these at once must open");

    let global_position = sigmf
        .metadata
        .global
        .geolocation
        .as_ref()
        .expect("the Global fallback position");
    assert_eq!(global_position.longitude, 14.5053);
    assert_eq!(global_position.latitude, -22.9576);
    assert_eq!(global_position.altitude, None);

    let capture_position = sigmf.metadata.captures[0]
        .geolocation
        .as_ref()
        .expect("the preferred Captures position");
    assert_eq!(capture_position.altitude, Some(7.0));

    let declared = sigmf
        .metadata
        .global
        .extensions
        .as_ref()
        .expect("the declared extension list");
    assert_eq!(declared[0].name, "antenna");
    assert!(
        declared[0].optional,
        "antenna is descriptive: a reader that skips it still gets every sample"
    );

    assert_eq!(
        sigmf
            .metadata
            .global
            .get_extension::<AntennaGlobal>()
            .expect("the antenna keys are well-formed"),
        Some(AntennaGlobal {
            model: "Wellbrook ALA1530".to_string(),
            ..Default::default()
        })
    );

    assert_round_trip_is_lossless("realistic_recording.sigmf-meta");
}

/// A geolocation's GeoJSON Foreign Members survive a round-trip.
///
/// RFC 7946 section 6.1 permits arbitrary extra members on a GeoJSON object, and
/// the schema does not close `core:geolocation` against them — it invites them, in
/// its own words, for "position valid indication, GNSS SV counts, dillution of
/// precision, accuracy". So a typed Point that models only `type`, `coordinates`,
/// and `bbox` reads a real file, drops the fields it did not expect, and reports
/// success — the same defect as the missing capture-scope catch-all, one scope
/// further down.
///
/// This test exists because that is the shape a typed geolocation naturally takes
/// if you write it from the schema's `properties` list and stop there.
#[test]
fn geolocation_foreign_members_survive_a_round_trip() {
    assert_round_trip_is_lossless("geolocation_foreign_members.sigmf-meta");

    let written = round_trip("geolocation_foreign_members.sigmf-meta");
    let geolocation = &written["captures"][0]["core:geolocation"];
    assert_eq!(geolocation["gnss:satellites"], json!(11));
    assert_eq!(geolocation["gnss:hdop"], json!(0.8));
    assert_eq!(
        geolocation["bbox"],
        json!([14.5052, -22.9577, 14.5054, -22.9575])
    );
}

/// `core:collection` lands in the `collection` field.
///
/// It once landed in `extensions`, because both fields carried the same
/// `#[serde(rename = "core:collection")]` and serde binds the first — leaving
/// `collection` unreachable and permanently `None`. Nothing warned: rustc and
/// clippy were both silent, because serde's derive expands the field-name match
/// into generated code the `unreachable_patterns` lint never saw.
///
/// This test asserts against the *typed field* rather than a round trip, and that
/// is deliberate: a round trip passed with the bug present, because the value
/// serialized back out under the same wrong name. Fidelity tests cannot see a
/// value that is merely in the wrong place.
///
/// The field is worth this care. `core:collection` associates a Recording with a
/// `.sigmf-collection`, and the schema recommends Collections over
/// `core:num_channels` for multi-channel IQ — exactly the shape a six-band
/// receiver needs.
#[test]
fn collection_field_binds_the_core_collection_key() {
    let sigmf = SigMF::from_file(fixture_path("collection.sigmf-meta")).expect("must open");
    assert_eq!(
        sigmf.metadata.global.collection,
        Some("walvisbay-2026-07-16T09-14-22".to_string()),
        "core:collection must arrive in the `collection` field"
    );
}

/// Capture-scope and annotation-scope keys survive a round trip.
///
/// `CaptureMetadata` and `AnnotationMetadata` once had no `#[serde(flatten)]`
/// catch-all where `GlobalMetadata` had one, so every key they did not model was
/// dropped on read and absent on write — while the write reported success.
///
/// That was a data-integrity bug rather than a conformance bug, and the distinction
/// is worth keeping in view. A reader that cannot see a field is inconvenient. A
/// writer that deletes a field it could not see, and tells you it succeeded, is
/// something else.
#[test]
fn scoped_extension_keys_survive_a_round_trip() {
    assert_round_trip_is_lossless("scoped_extension_keys.sigmf-meta");

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

/// Writing extension data also declares the extension.
///
/// The schema is explicit that `core:extensions` is how a reader learns it must
/// support a namespace before parsing. `set_extension` used to write
/// `antenna:model` and never touch the declaration, so the crate emitted files
/// that *used* an extension without *declaring* it. The declared-extensions list
/// and the extension accessors were entirely unconnected code — which is plausibly
/// why nobody noticed the field was dead: nothing in the crate ever read it.
#[test]
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
