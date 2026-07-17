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
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use sigmf::num_complex::Complex;
use sigmf::*;
use tempfile::TempDir;

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
        .to_json()
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
/// caller happened to overwrite both. `GlobalMetadata::describing` cannot do that:
/// it takes a parsed datatype and supplies the version itself.
#[test]
fn a_recording_built_through_the_constructor_validates() {
    let metadata = Metadata {
        global: GlobalMetadata::describing("cf32_le".parse().expect("cf32_le is a valid datatype")),
        captures: vec![],
        annotations: vec![],
    };

    let written: Value =
        serde_json::from_str(&metadata.to_json().expect("serialize")).expect("output must be JSON");

    assert_valid(
        &written,
        "a recording built through GlobalMetadata::describing",
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

    let written: Value = serde_json::from_str(&sigmf.metadata.to_json().expect("serialize"))
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

/// The oracle, pointed at our own output.
///
/// Everything above judges fixtures — files someone else wrote — or metadata this
/// crate read and handed straight back. These tests judge recordings the crate
/// *originates*: samples in, two files out, and the specification's own schema
/// asked whether what landed is a SigMF recording.
///
/// One claim runs through all of them, and it is the claim the typed write path
/// exists to make: `core:datatype` and `core:sha512` describe the Dataset that was
/// actually written, and cannot be talked into describing anything else.
/// The document for a Recording of a coast-station DSC watch on 16 MHz at
/// 32 kSa/s — the case this crate was written to serve, and the one
/// `realistic_recording.sigmf-meta` describes.
///
/// `datatype` is the caller's *claim*, and it is a parameter because several tests
/// hand in a claim that is false and assert the written file contradicts it.
fn a_dsc_watch_metadata(datatype: &str) -> Metadata {
    let mut global = GlobalMetadata::describing(datatype.parse().expect("a valid datatype"));
    global.sample_rate = Some(32_000.0);
    global.recorder = Some("winradio-agent".to_string());

    let mut capture = CaptureMetadata::new(0);
    capture.frequency = Some(16_804_500.0);
    capture.datetime = Some("2026-07-16T09:14:22.000Z".to_string());

    Metadata {
        global,
        captures: vec![capture],
        annotations: vec![],
    }
}

/// Samples with no two components alike, so that a test can tell in-phase from
/// quadrature, and one byte order from the other.
fn dsc_samples() -> Vec<Complex<f32>> {
    vec![
        Complex::new(1.0, -2.0),
        Complex::new(0.5, 0.25),
        Complex::new(-1.5, 3.0),
    ]
}

/// Where a Recording's two files land, worked out independently of the crate's own
/// helper so that a mistake in that helper cannot hide behind this one.
fn sibling(basename: &Path, extension: &str) -> PathBuf {
    PathBuf::from(format!("{}{extension}", basename.display()))
}

mod write_path {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
    }

    /// The datatype in the file describes the bytes in the file, whatever the
    /// caller believed when they built the metadata.
    ///
    /// This is what the whole typed write path is for. The Global handed over here
    /// says `ri16_le` — real 16-bit integers, two bytes a sample — and the samples
    /// are complex 32-bit floats, eight bytes a sample. Both cannot be true of the
    /// bytes on disk, and it is not the caller's claim that survives.
    #[test]
    fn the_datatype_written_describes_the_samples_not_the_callers_claim() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        let recording = RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("ri16_le"))
            .to_file(&basename)
            .expect("writing must succeed");

        let written = read_json(&sibling(&basename, ".sigmf-meta"));
        assert_eq!(
            written["global"]["core:datatype"],
            json!("cf32_le"),
            "the file must describe the samples it holds, not the claim it was handed"
        );
        assert_eq!(
            recording.metadata.global.datatype.to_string(),
            "cf32_le",
            "and the recording handed back must agree with the file, rather than \
             keeping a claim the caller could go on to write somewhere else"
        );
    }

    /// The specification's own schema, asked about a recording this crate made.
    #[test]
    fn a_written_recording_validates_against_the_spec_schema() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect("writing must succeed");

        assert_valid(
            &read_json(&sibling(&basename, ".sigmf-meta")),
            "a recording written by this crate",
        );
    }

    /// Samples in, samples out: what was written is what was handed over.
    #[test]
    fn a_written_recording_reads_back_with_its_metadata_and_samples_intact() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect("writing must succeed");

        let reopened =
            SigMF::from_file(sibling(&basename, ".sigmf-meta")).expect("the recording must reopen");
        let global = &reopened.metadata.global;
        assert_eq!(global.datatype.to_string(), "cf32_le");
        assert_eq!(global.sample_rate, Some(32_000.0));
        assert_eq!(global.recorder.as_deref(), Some("winradio-agent"));
        assert_eq!(global.version, SIGMF_VERSION);
        assert_eq!(
            reopened.metadata.captures[0].frequency,
            Some(16_804_500.0),
            "the capture's centre frequency must survive"
        );

        // Interleaved, in-phase first, little-endian — recomputed here rather than
        // borrowed from the crate that is on trial.
        let expected: Vec<u8> = samples
            .iter()
            .flat_map(|s| [s.re.to_le_bytes(), s.im.to_le_bytes()])
            .flatten()
            .collect();
        assert_eq!(
            fs::read(sibling(&basename, ".sigmf-data")).expect("a dataset"),
            expected,
            "the dataset must be the samples, interleaved and little-endian"
        );
    }

    /// Big-endian samples are written big-endian, and the datatype says so.
    #[test]
    fn a_big_endian_recording_says_it_is_big_endian_and_is() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .endianness(Endianness::BigEndian)
            .to_file(&basename)
            .expect("writing must succeed");

        let written = read_json(&sibling(&basename, ".sigmf-meta"));
        assert_eq!(written["global"]["core:datatype"], json!("cf32_be"));

        let expected: Vec<u8> = samples
            .iter()
            .flat_map(|s| [s.re.to_be_bytes(), s.im.to_be_bytes()])
            .flatten()
            .collect();
        assert_eq!(
            fs::read(sibling(&basename, ".sigmf-data")).expect("a dataset"),
            expected
        );
    }

    /// `core:sha512` hashes the Dataset that is on disk, not the buffer we meant to
    /// put there.
    ///
    /// The length assertion is not redundant with [`assert_valid`]: the schema's
    /// pattern for this field is `^[0-9a-fA-F]{128}` — anchored at the start and
    /// never closed — so the oracle accepts 128 hex digits with anything at all
    /// appended. It cannot catch a hash that is too long, and this can.
    #[test]
    fn the_written_sha512_is_the_hash_of_the_dataset_on_disk() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect("writing must succeed");

        let on_disk = fs::read(sibling(&basename, ".sigmf-data")).expect("a dataset");
        let written = read_json(&sibling(&basename, ".sigmf-meta"));
        let hash = written["global"]["core:sha512"]
            .as_str()
            .expect("a recording must be written with a checksum by default");

        assert_eq!(hash, hex(&Sha512::digest(&on_disk)));
        assert_eq!(hash.len(), 128, "a SHA-512 is 128 hex digits: {hash}");
    }

    /// Turning the checksum off omits the field rather than leaving a hash of
    /// something else behind.
    ///
    /// The Global here arrives carrying a hash — as one read from an existing file
    /// would — and is then written with different samples. Preserving that hash
    /// would produce a recording that fails its own integrity check, which is the
    /// same class of lie as a datatype that does not match its bytes.
    #[test]
    fn writing_without_a_checksum_clears_a_stale_hash_rather_than_keeping_it() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        let mut metadata = a_dsc_watch_metadata("cf32_le");
        metadata.global.sha512 = Some("0".repeat(128));
        RecordingWriter::with_metadata(&samples, metadata)
            .checksum(false)
            .to_file(&basename)
            .expect("writing must succeed");

        let written = read_json(&sibling(&basename, ".sigmf-meta"));
        assert_eq!(
            written["global"].get("core:sha512"),
            None,
            "a hash of a dataset that no longer exists must not survive the write"
        );
        assert_valid(&written, "a recording written without a checksum");
    }

    /// A multi-channel Dataset is refused, and the refusal points at Collections.
    ///
    /// Not a limitation this crate invented: a `&[S]` has nowhere to say where one
    /// channel ends and the next begins, so honouring `core:num_channels > 1` would
    /// mean writing a datatype that describes something other than the bytes. The
    /// specification's own advice is to use Collections instead, and the error says
    /// so rather than reporting a bare refusal.
    #[test]
    fn a_multi_channel_recording_is_refused_and_the_error_names_collections() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        let mut metadata = a_dsc_watch_metadata("cf32_le");
        metadata.global.num_channels = Some(2);
        let error = RecordingWriter::with_metadata(&samples, metadata)
            .to_file(&basename)
            .expect_err("two channels must be refused");

        let message = error.to_string();
        assert!(
            message.contains("Collections"),
            "the error must name the alternative the specification recommends: {message}"
        );
        assert!(
            !sibling(&basename, ".sigmf-data").exists(),
            "a refused write must not leave a dataset behind"
        );

        // One channel is what a typed buffer is, whether or not it is spelled out.
        let mut metadata = a_dsc_watch_metadata("cf32_le");
        metadata.global.num_channels = Some(1);
        RecordingWriter::with_metadata(&samples, metadata)
            .to_file(&basename)
            .expect("one channel must be accepted");
    }

    /// A write interrupted between the two files leaves the Dataset, not the
    /// sidecar.
    ///
    /// A `.sigmf-data` with no `.sigmf-meta` is visibly unfinished; a `.sigmf-meta`
    /// describing a truncated `.sigmf-data` looks valid and lies about its length,
    /// which is worse than an obvious failure. On a receiver that loses power
    /// mid-capture that is the whole difference between a recording you can trust
    /// and one you can only hope about.
    ///
    /// The interruption is staged by putting a directory where the Metadata file
    /// belongs, so its write cannot succeed. The Dataset can only be on disk
    /// afterwards if it was written first.
    #[test]
    fn the_dataset_is_written_before_the_metadata() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");
        fs::create_dir(sibling(&basename, ".sigmf-meta")).expect("blocking the metadata path");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect_err("the metadata write must fail");

        assert!(
            sibling(&basename, ".sigmf-data").exists(),
            "the dataset must already be on disk when the metadata write fails"
        );
    }

    /// A basename with a dot in it keeps all of itself.
    ///
    /// `dsc_16804.5kHz` is an ordinary name for an RF capture, and the two files of
    /// a Recording are defined as sharing a basename — so the extension is appended
    /// to whatever the caller gave, never substituted into it. Reaching for
    /// `Path::set_extension` here would read `.5kHz` as an extension to replace and
    /// quietly write `dsc_16804.sigmf-data` instead, leaving both files misnamed and
    /// two captures an hour apart free to collide.
    ///
    /// Every other test in this module uses an undotted basename, where the wrong
    /// implementation and the right one agree. This is the only one that can tell
    /// them apart.
    #[test]
    fn a_basename_containing_a_dot_survives_intact() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_16804.5kHz");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect("writing must succeed");

        assert!(
            sibling(&basename, ".sigmf-data").exists(),
            "the dataset must be dsc_16804.5kHz.sigmf-data, not dsc_16804.sigmf-data"
        );
        assert!(
            sibling(&basename, ".sigmf-meta").exists(),
            "the metadata must be dsc_16804.5kHz.sigmf-meta, not dsc_16804.sigmf-meta"
        );
    }

    /// Every type in the sample vocabulary maps to the datatype the specification
    /// spells it with.
    ///
    /// The width test below cannot see a mistake between two types of the same
    /// width — `u32` declared `i32`, or either declared `f32` — and each of those
    /// misreads a Dataset as thoroughly as a wrong width does. This pins all
    /// sixteen by name.
    #[test]
    fn every_sample_type_maps_to_the_datatype_the_specification_spells_it() {
        let le = Endianness::LittleEndian;
        assert_eq!(DataFormat::of::<f32>(le).to_string(), "rf32_le");
        assert_eq!(DataFormat::of::<f64>(le).to_string(), "rf64_le");
        assert_eq!(DataFormat::of::<i32>(le).to_string(), "ri32_le");
        assert_eq!(DataFormat::of::<i16>(le).to_string(), "ri16_le");
        assert_eq!(DataFormat::of::<u32>(le).to_string(), "ru32_le");
        assert_eq!(DataFormat::of::<u16>(le).to_string(), "ru16_le");
        assert_eq!(DataFormat::of::<i8>(le).to_string(), "ri8");
        assert_eq!(DataFormat::of::<u8>(le).to_string(), "ru8");

        assert_eq!(DataFormat::of::<Complex<f32>>(le).to_string(), "cf32_le");
        assert_eq!(DataFormat::of::<Complex<f64>>(le).to_string(), "cf64_le");
        assert_eq!(DataFormat::of::<Complex<i32>>(le).to_string(), "ci32_le");
        assert_eq!(DataFormat::of::<Complex<i16>>(le).to_string(), "ci16_le");
        assert_eq!(DataFormat::of::<Complex<u32>>(le).to_string(), "cu32_le");
        assert_eq!(DataFormat::of::<Complex<u16>>(le).to_string(), "cu16_le");
        assert_eq!(DataFormat::of::<Complex<i8>>(le).to_string(), "ci8");
        assert_eq!(DataFormat::of::<Complex<u8>>(le).to_string(), "cu8");

        // Byte order is the caller's to choose, except where the specification says
        // there is none to choose: one byte has no order, `ri8_le` is not a
        // datatype, and `DataFormat`'s parser rejects it — so the writer must never
        // emit it.
        let be = Endianness::BigEndian;
        assert_eq!(DataFormat::of::<f32>(be).to_string(), "rf32_be");
        assert_eq!(DataFormat::of::<Complex<i16>>(be).to_string(), "ci16_be");
        assert_eq!(DataFormat::of::<i8>(be).to_string(), "ri8");
        assert_eq!(DataFormat::of::<Complex<u8>>(be).to_string(), "cu8");
    }

    /// Every sample type writes exactly the number of bytes its datatype declares.
    ///
    /// `impl_sample!` pairs a Rust type with a `DataType` by hand, sixteen times,
    /// and nothing in the type system checks that pairing — the compiler will
    /// cheerfully believe an `f64` is four bytes wide if the macro says so. A slip
    /// there produces exactly the silent misinterpretation the sealed trait exists
    /// to prevent, introduced by the mechanism meant to prevent it. So the width is
    /// checked twice: against the Rust type, and against what reaches the disk.
    #[test]
    fn every_sample_type_writes_exactly_the_width_it_declares() {
        fn check<S: Sample>(dir: &TempDir, sample: S, label: &str) {
            let declared = DataFormat::of::<S>(Endianness::LittleEndian).size();
            assert_eq!(
                declared as usize,
                std::mem::size_of::<S>(),
                "{label}: core:datatype declares {declared} bytes a sample, the Rust type is {}",
                std::mem::size_of::<S>()
            );

            let basename = dir.path().join(label);
            RecordingWriter::new(&[sample], 32_000.0)
                .to_file(&basename)
                .expect("writing must succeed");

            let on_disk = fs::metadata(sibling(&basename, ".sigmf-data"))
                .expect("a dataset")
                .len();
            assert_eq!(
                on_disk, declared,
                "{label}: one sample declares {declared} bytes but wrote {on_disk}"
            );
        }

        let dir = TempDir::new().expect("a temp dir");
        check(&dir, 1.0f32, "rf32");
        check(&dir, 1.0f64, "rf64");
        check(&dir, 1i32, "ri32");
        check(&dir, 1i16, "ri16");
        check(&dir, 1u32, "ru32");
        check(&dir, 1u16, "ru16");
        check(&dir, 1i8, "ri8");
        check(&dir, 1u8, "ru8");
        check(&dir, Complex::new(1.0f32, 2.0), "cf32");
        check(&dir, Complex::new(1.0f64, 2.0), "cf64");
        check(&dir, Complex::new(1i32, 2), "ci32");
        check(&dir, Complex::new(1i16, 2), "ci16");
        check(&dir, Complex::new(1u32, 2), "cu32");
        check(&dir, Complex::new(1u16, 2), "cu16");
        check(&dir, Complex::new(1i8, 2), "ci8");
        check(&dir, Complex::new(1u8, 2), "cu8");
    }
}

/// The same guarantee, arriving from the other direction.
///
/// `core:datatype` is the Recording's own account of what its bytes mean, and a
/// reader that takes the caller's word instead fails exactly as a writer would:
/// silently, with plausible numbers. These tests judge what the crate does when the
/// two disagree — and what it does to find a Dataset before it can read one.
mod read_path {
    use super::*;

    /// The Dataset of `sample.sigmf-meta`, decoded by hand.
    ///
    /// This fixture predates every line of the read path and was not produced by it:
    /// 64 bytes of `rf32_le` holding the integers 0 through 15. That independence is
    /// the point. A round-trip test cannot tell a correct codec from one whose
    /// encode and decode are wrong in the same direction; bytes someone else wrote
    /// can.
    fn sample_fixture_samples() -> Vec<f32> {
        (0..16).map(|n| n as f32).collect()
    }

    /// Byte boundaries, on the fixture that used to answer `(0, 0)` for them.
    ///
    /// The Dataset is 64 bytes and the sole segment starts at sample 0, so the
    /// answer is the whole file. It used to be `(0, 0)` — not because anything
    /// computed zero, but because `from_file` computed nothing and the field held
    /// its default. `(0, 0)` is at least visibly absurd; the arithmetic standing in
    /// for it can be subtly wrong instead, which is why the answer now comes from a
    /// method that will not speak without the Dataset's length.
    #[test]
    fn the_boundaries_of_a_real_recording_cover_its_real_dataset() {
        let sigmf = SigMF::from_file(fixture_path("sample.sigmf-meta")).expect("must open");

        let dataset_len = fs::metadata(fixture_path("sample.sigmf-data"))
            .expect("the fixture's dataset must exist")
            .len();
        assert_eq!(
            dataset_len, 64,
            "the fixture must be the one this test describes"
        );

        assert_eq!(
            sigmf.capture_boundaries().expect("boundaries must compute"),
            vec![0..64],
        );
    }

    /// Samples out of a file this crate did not write.
    #[test]
    fn a_real_recording_decodes_to_the_samples_it_holds() {
        let sigmf = SigMF::from_file(fixture_path("sample.sigmf-meta")).expect("must open");

        assert_eq!(
            sigmf.samples::<f32>().expect("rf32_le must read as f32"),
            sample_fixture_samples(),
        );
    }

    /// The read path refuses what the write path refuses, and names both formats.
    #[test]
    fn reading_a_dataset_as_the_wrong_type_is_refused_and_names_both_formats() {
        let sigmf = SigMF::from_file(fixture_path("sample.sigmf-meta")).expect("must open");

        // The fixture is `rf32_le`: 16 real floats. Asked for complex 16-bit
        // integers, those same 64 bytes would "decode" to 16 samples of noise with
        // nothing anywhere reporting a problem.
        let err = sigmf
            .samples::<Complex<i16>>()
            .expect_err("ri16_le bytes must not be conjured out of an rf32_le dataset");
        let message = err.to_string();
        assert!(
            message.contains("rf32_le"),
            "the error must name what the Recording says it is: {message}"
        );
        assert!(
            message.contains("ci16_le"),
            "the error must name what was asked for: {message}"
        );
    }

    /// Reading a Recording's metadata does not read its Dataset.
    ///
    /// Not a statement about performance. The only way to get correct boundaries
    /// used to be `Metadata::from_str`, which demanded the whole Dataset as a
    /// `&Vec<u8>` in order to read `.len()` off it — so metadata for a
    /// hundred-gigabyte Recording cost a hundred gigabytes of memory. Nothing here
    /// could run at all if that were still so: this fixture's Dataset does not
    /// exist.
    #[test]
    fn metadata_opens_without_a_dataset_to_open() {
        let dataset = fixture_path("realistic_recording.sigmf-data");
        assert!(
            !dataset.exists(),
            "this test is vacuous unless {} is absent",
            dataset.display()
        );

        let sigmf = SigMF::from_file(fixture_path("realistic_recording.sigmf-meta"))
            .expect("metadata must open with no dataset beside it");
        assert_eq!(sigmf.metadata.global.sample_rate, Some(32_000.0));

        // And the absence is reported when the Dataset is finally wanted, rather
        // than papered over with an empty result.
        assert!(
            sigmf.samples::<Complex<f32>>().is_err(),
            "samples must not be invented for a dataset that is not there"
        );
    }

    /// A Recording written and then read back, typed on both ends.
    #[test]
    fn samples_survive_a_write_and_a_read() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect("writing must succeed");

        let reopened =
            SigMF::from_file(sibling(&basename, ".sigmf-meta")).expect("the recording must reopen");
        assert_eq!(
            reopened
                .samples::<Complex<f32>>()
                .expect("what we wrote must read back"),
            samples,
        );
    }

    /// Byte order survives the trip.
    ///
    /// Both halves consult `core:datatype`, so a codec ignoring byte order entirely
    /// would pass a little-endian round-trip. This writes big-endian and checks the
    /// bytes on disk really are big-endian before reading them back.
    #[test]
    fn a_big_endian_recording_reads_back_as_what_was_written() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .endianness(Endianness::BigEndian)
            .to_file(&basename)
            .expect("writing must succeed");

        let written = fs::read(sibling(&basename, ".sigmf-data")).expect("dataset");
        assert_eq!(
            &written[..4],
            &samples[0].re.to_be_bytes(),
            "the dataset must actually be big-endian, or this test proves nothing"
        );

        let reopened =
            SigMF::from_file(sibling(&basename, ".sigmf-meta")).expect("the recording must reopen");
        assert_eq!(
            reopened.samples::<Complex<f32>>().expect("must read back"),
            samples,
        );
    }

    /// A Recording whose basename has a dot in it finds its own Dataset.
    ///
    /// The sibling `.sigmf-data` is named by replacing the `.sigmf-meta` extension,
    /// which is safe only because the segment being replaced is the one just
    /// matched. `dsc_16804.5kHz` is the case that tells that apart from an
    /// implementation reaching for `file_stem`.
    #[test]
    fn a_dotted_basename_still_finds_its_dataset() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_16804.5kHz");

        let samples = dsc_samples();
        RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect("writing must succeed");

        let reopened =
            SigMF::from_file(sibling(&basename, ".sigmf-meta")).expect("the recording must reopen");
        assert_eq!(
            reopened.samples::<Complex<f32>>().expect("must read back"),
            samples,
            "the dataset beside dsc_16804.5kHz.sigmf-meta is dsc_16804.5kHz.sigmf-data"
        );
    }

    /// `core:metadata_only` says there is no Dataset, and is believed.
    #[test]
    fn a_metadata_only_recording_reports_that_it_has_no_dataset() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        // Write a real Recording, then re-describe it as metadata-only. The Dataset
        // is still sitting there on disk, so anything that goes looking will find
        // it — which is what makes this a test of the field rather than of the file.
        let samples = dsc_samples();
        let recording = RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect("writing must succeed");
        assert!(sibling(&basename, ".sigmf-data").exists());

        let mut metadata = recording.metadata;
        metadata.global.metadata_only = Some(true);
        fs::write(
            sibling(&basename, ".sigmf-meta"),
            metadata.to_json().expect("serialize"),
        )
        .expect("rewriting the sidecar");

        let reopened = SigMF::from_file(sibling(&basename, ".sigmf-meta"))
            .expect("a metadata-only recording still opens");
        let err = reopened
            .samples::<Complex<f32>>()
            .expect_err("a recording saying it has no dataset must not read the one beside it");
        assert!(
            err.to_string().contains("metadata_only"),
            "the error should name the field that decided this: {err}"
        );
    }

    /// `core:dataset` names a file beside the Metadata file, and nothing else.
    ///
    /// The schema: this field "only includes the filename, not directory", and the
    /// Dataset "must be in the same directory as the .sigmf-meta file". A Metadata
    /// file is a document that may have come from anywhere, so a `core:dataset` of
    /// `../../../etc/passwd` is a document choosing what its reader opens. The
    /// specification's own rule is the defence; this checks the crate enforces it
    /// rather than resolving it and hoping.
    #[test]
    fn a_core_dataset_outside_the_directory_is_refused() {
        let dir = TempDir::new().expect("a temp dir");
        let basename = dir.path().join("dsc_watch");

        let samples = dsc_samples();
        let recording = RecordingWriter::with_metadata(&samples, a_dsc_watch_metadata("cf32_le"))
            .to_file(&basename)
            .expect("writing must succeed");

        let mut metadata = recording.metadata;
        metadata.global.dataset = Some("../elsewhere/secrets.bin".to_string());
        fs::write(
            sibling(&basename, ".sigmf-meta"),
            metadata.to_json().expect("serialize"),
        )
        .expect("rewriting the sidecar");

        let err = SigMF::from_file(sibling(&basename, ".sigmf-meta"))
            .expect_err("a core:dataset with a directory component must be refused");
        assert!(
            err.to_string().contains("only includes the filename"),
            "the error should quote the rule it enforces: {err}"
        );
    }

    /// A Non-Conforming Dataset is read from the file `core:dataset` names.
    #[test]
    fn a_core_dataset_names_the_file_that_is_read() {
        let dir = TempDir::new().expect("a temp dir");

        // An NCD: the samples live under a name of the recorder's choosing, and the
        // sidecar is the only thing tying the two together.
        let samples = dsc_samples();
        let dataset: Vec<u8> = samples
            .iter()
            .flat_map(|s| [s.re.to_le_bytes(), s.im.to_le_bytes()])
            .flatten()
            .collect();
        fs::write(dir.path().join("capture.iq"), &dataset).expect("writing the dataset");

        let mut metadata = a_dsc_watch_metadata("cf32_le");
        metadata.global.dataset = Some("capture.iq".to_string());
        let sidecar = dir.path().join("dsc_watch.sigmf-meta");
        fs::write(&sidecar, metadata.to_json().expect("serialize")).expect("writing the sidecar");

        let reopened = SigMF::from_file(&sidecar).expect("an NCD recording must open");
        assert_eq!(
            reopened
                .samples::<Complex<f32>>()
                .expect("the named dataset must be the one read"),
            samples,
        );
    }

    /// Every sample type reads back as what it was written as.
    ///
    /// Each of the sixteen has its width and its datatype pinned by the write path's
    /// own tests. This pins that decode is `encode`'s inverse, which is a different
    /// claim failing in a different way: a transposed in-phase/quadrature pair has
    /// the right width and the right datatype.
    #[test]
    fn every_sample_type_reads_back_as_what_it_was_written_as() {
        fn check<S: Sample + std::fmt::Debug + PartialEq>(
            dir: &TempDir,
            samples: &[S],
            name: &str,
        ) {
            let basename = dir.path().join(name);
            let mut writer = RecordingWriter::new(samples, 32_000.0);
            writer.captures_mut().push(CaptureMetadata::new(0));
            writer
                .to_file(&basename)
                .unwrap_or_else(|e| panic!("writing {name}: {e}"));

            let reopened = SigMF::from_file(sibling(&basename, ".sigmf-meta"))
                .unwrap_or_else(|e| panic!("reopening {name}: {e}"));
            assert_eq!(
                reopened
                    .samples::<S>()
                    .unwrap_or_else(|e| panic!("reading {name}: {e}")),
                samples,
                "{name} must read back as what it was written as"
            );
        }

        let dir = TempDir::new().expect("a temp dir");
        // No two components alike, and none symmetric, so that a transposition or a
        // byte-order slip cannot land back on the value it started from.
        check(&dir, &[1.0f32, -2.5, 3.25], "rf32");
        check(&dir, &[1.0f64, -2.5, 3.25], "rf64");
        check(&dir, &[1i32, -2, 3], "ri32");
        check(&dir, &[1i16, -2, 3], "ri16");
        check(&dir, &[1u32, 2, 3], "ru32");
        check(&dir, &[1u16, 2, 3], "ru16");
        check(&dir, &[1i8, -2, 3], "ri8");
        check(&dir, &[1u8, 2, 3], "ru8");
        check(&dir, &[Complex::new(1.0f32, -2.5)], "cf32");
        check(&dir, &[Complex::new(1.0f64, -2.5)], "cf64");
        check(&dir, &[Complex::new(1i32, -2)], "ci32");
        check(&dir, &[Complex::new(1i16, -2)], "ci16");
        check(&dir, &[Complex::new(1u32, 2)], "cu32");
        check(&dir, &[Complex::new(1u16, 2)], "cu16");
        check(&dir, &[Complex::new(1i8, -2)], "ci8");
        check(&dir, &[Complex::new(1u8, 2)], "cu8");
    }
}
