//! What a caller can do with a failure.
//!
//! These tests live outside the crate on purpose. Everything they assert — that
//! the types are reachable at the crate root, that `?` carries every failure into
//! one type, that a cause survives into the chain — is a property of the *public*
//! surface, and a test inside `src/` is not subject to it: privacy, re-exports and
//! `#[non_exhaustive]` are all invisible from within. This file sees the crate the
//! way a dependent sees it, which is the only vantage point from which these
//! questions have answers.
//!
//! The imports are the first assertion. `use sigmf::*` reaching `Metadata` at all
//! is what pins the crate-root re-export; before 0.2.0 this had to be
//! `sigmf::sigmf::Metadata`.

use std::error::Error as _;
use std::path::Path;

use serde::{Serialize, Serializer};
use sigmf::num_complex::Complex;
use sigmf::*;
use tempfile::TempDir;

/// An extension whose `Serialize` fails, for reaching the error path that a
/// well-behaved type never reaches.
struct Unserializable;

impl Serialize for Unserializable {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("this type refuses to serialize"))
    }
}

impl GlobalExtension for Unserializable {
    fn namespace() -> String {
        "unserializable".to_string()
    }
    fn version() -> String {
        "1.0.0".to_string()
    }
}

/// An extension that serializes to a JSON string rather than an object.
///
/// A newtype struct serializes as the value it wraps, so this is what a caller
/// gets by wrapping the wrong thing — not a contrived failure but a plausible one.
#[derive(Serialize)]
struct NotAnObject(String);

impl GlobalExtension for NotAnObject {
    fn namespace() -> String {
        "notanobject".to_string()
    }
    fn version() -> String {
        "1.0.0".to_string()
    }
}

fn a_global() -> GlobalMetadata {
    GlobalMetadata::describing("cf32_le".parse().expect("cf32_le is a valid datatype"))
}

/// The cause of a serde-caused failure is reachable, not just printable.
///
/// `source()` returned `None` unconditionally until 0.2.0, including for the
/// variant that wrapped a real `serde_json::Error`. The message was never lost —
/// `Display` forwarded it verbatim, so a printed report always read correctly —
/// but a caller that wanted to *inspect* the cause rather than print it, which is
/// what walking a chain is for, found nothing there to inspect. `line()` and
/// `classify()` are on `serde_json::Error` and on nothing else, so a chain that
/// drops the type drops them.
#[test]
fn a_serde_caused_error_keeps_the_serde_error_in_its_chain() {
    let mut global = a_global();
    let err = global
        .set_extension(Unserializable)
        .expect_err("a type whose Serialize fails cannot be stored");

    let source = err.source().expect("the serde error is the cause");
    let serde_error = source
        .downcast_ref::<serde_json::Error>()
        .expect("and it is reachable as itself, not merely as a string");
    assert!(serde_error.is_data(), "serde classified it as a data error");

    // The context the wrapper adds is the namespace: the serde error knows a type
    // refused to serialize, but not which extension the caller was writing.
    assert!(
        err.to_string().contains("unserializable"),
        "the message names the namespace: {err}"
    );
}

/// The `found` field says what arrived, so the caller need not guess.
#[test]
fn an_extension_that_is_not_an_object_is_refused_and_names_what_it_got() {
    let mut global = a_global();
    let err = global
        .set_extension(NotAnObject("a bare string".to_string()))
        .expect_err("extension data must be a JSON object");

    match err {
        MetadataError::ExtensionNotAnObject { namespace, found } => {
            assert_eq!(namespace, "notanobject");
            assert_eq!(found, "string");
        }
        other => panic!("expected ExtensionNotAnObject, got {other:?}"),
    }
}

/// A failure to read names the file it failed to read.
///
/// `std::io::Error` carries no path — "No such file or directory" is its whole
/// message — and a Recording is two files, so an error without one leaves the
/// reader unable to tell which half is missing.
#[test]
fn a_file_that_cannot_be_opened_is_named_in_the_error() {
    let err =
        SigMF::from_file("/nonexistent/dsc_watch.sigmf-meta").expect_err("there is no such file");

    match &err {
        Error::Io { path, source } => {
            assert_eq!(path, Path::new("/nonexistent/dsc_watch.sigmf-meta"));
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected Error::Io, got {other:?}"),
    }

    assert!(
        err.to_string().contains("dsc_watch.sigmf-meta"),
        "and the rendered message names it too: {err}"
    );
    assert!(
        err.source().is_some(),
        "the io::Error stays reachable as the cause"
    );
}

/// `core:trailing_bytes` beyond the end of the Dataset reports both numbers.
///
/// This was an `Internal(String)` — a variant whose only content was prose, which
/// a caller could match on but not read. Both numbers are now fields.
#[test]
fn trailing_bytes_beyond_the_dataset_report_both_numbers() {
    let metadata = Metadata {
        global: {
            let mut global = a_global();
            global.trailing_bytes = Some(4096);
            global
        },
        captures: vec![],
        annotations: vec![],
    };

    let err = metadata
        .capture_boundaries(64)
        .expect_err("4096 trailing bytes cannot come out of a 64-byte Dataset");

    match err {
        MetadataError::TrailingBytesExceedDataset {
            trailing,
            dataset_len,
        } => {
            assert_eq!(trailing, 4096);
            assert_eq!(dataset_len, 64);
        }
        other => panic!("expected TrailingBytesExceedDataset, got {other:?}"),
    }
}

/// A `core:sample_start` too large to be a byte offset is refused, not wrapped.
#[test]
fn a_sample_start_past_the_end_of_addressable_bytes_is_refused() {
    let metadata: Metadata = serde_json::from_value(serde_json::json!({
        "global": { "core:datatype": "cf32_le", "core:version": SIGMF_VERSION },
        // Eight bytes per cf32_le sample, so this sample index is a byte offset
        // that does not fit in a u64. A release build would wrap it silently.
        "captures": [{ "core:sample_start": u64::MAX }],
        "annotations": [],
    }))
    .expect("the document is well-formed; it is the arithmetic that is not");

    match metadata
        .capture_boundaries(64)
        .expect_err("cannot be addressed")
    {
        MetadataError::SampleStartOutOfRange {
            index,
            sample_start,
        } => {
            assert_eq!(index, 0);
            assert_eq!(sample_start, u64::MAX);
        }
        other => panic!("expected SampleStartOutOfRange, got {other:?}"),
    }
}

/// Every fallible entry point converges on one error type — checked by the
/// compiler, not by an assertion.
///
/// The `?`s below are the whole test. Each one demands a
/// `From<ThatMethodsError> for sigmf::Error`, so this function compiles only while
/// every public fallible method returns something that converts into `Error`. A
/// method returning `Box<dyn Error>` has no such conversion and would break the
/// build here — which is what makes this the assertion for "no public signature
/// returns `Box<dyn Error>`", a property no runtime check can observe.
#[test]
fn every_fallible_method_converges_on_one_error_type() -> Result<(), Error> {
    let dir = TempDir::new().expect("a temporary directory");
    let basename = dir.path().join("convergence");

    let samples = [Complex::new(1.0f32, 0.0)];
    let mut writer = RecordingWriter::new(&samples);
    writer.global_mut().set_extension(AntennaGlobal {
        model: "Wellbrook ALA1530".to_string(),
        ..Default::default()
    })?; // MetadataError
    let recording = writer.to_file(&basename)?; // Error

    let json = recording.metadata.to_json()?; // serde_json::Error
    let parsed = Metadata::from_json(&json)?; // serde_json::Error
    parsed.capture_boundaries(8)?; // MetadataError

    let path = basename.with_file_name("convergence.sigmf-meta");
    let reopened = SigMF::from_file(&path)?; // Error
    reopened.capture_boundaries()?; // Error
    let samples: Vec<Complex<f32>> = reopened.samples()?; // Error

    assert_eq!(samples, vec![Complex::new(1.0f32, 0.0)]);
    Ok(())
}

/// The error types can cross a thread boundary and enter an `anyhow` chain.
///
/// This is the practical reason `Box<dyn Error>` had to go, and it is a compile-
/// time property that no test could otherwise state. `Box<dyn Error>` is neither
/// `Send` nor `Sync` and does not itself implement `Error`, so a caller could not
/// return one from a spawned task, and `anyhow`, whose blanket conversion is
/// `E: Error + Send + Sync + 'static`, would not take one either. A crate written
/// to be called from a receiver's async capture loop cannot ask its callers to
/// unwrap at the boundary.
#[test]
fn the_error_types_are_send_and_sync_and_static() {
    fn usable_with_anyhow<E: std::error::Error + Send + Sync + 'static>() {}

    usable_with_anyhow::<Error>();
    usable_with_anyhow::<MetadataError>();
    usable_with_anyhow::<ParseDataFormatError>();
}
