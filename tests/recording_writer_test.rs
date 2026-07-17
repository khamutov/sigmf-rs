//! Contracts of the write path.
//!
//! A [`RecordingWriter`] starts from the one thing a writer must have — the
//! samples — and derives everything derivable from them, most importantly
//! `core:datatype`. These tests pin that shape: no placeholder datatype to
//! invent, no claim that can reach the file, and a round-trip that carries the
//! whole document through.

use serde_json::{json, Value};
use sigmf::num_complex::Complex;
use sigmf::Endianness::BigEndian;
use sigmf::{
    CaptureMetadata, Error, GlobalMetadata, MetadataError, RecordingWriter, SigMF, SIGMF_VERSION,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Samples with no two components alike, so that a test can tell in-phase from
/// quadrature, and one byte order from the other.
fn dsc_samples() -> Vec<Complex<f32>> {
    vec![
        Complex::new(1.0, -2.0),
        Complex::new(0.5, 0.25),
        Complex::new(-1.5, 3.0),
    ]
}

/// Where a Recording's two files land, worked out independently of the crate's
/// own helper so that a mistake in that helper cannot hide behind this one.
fn sibling(basename: &Path, extension: &str) -> PathBuf {
    PathBuf::from(format!("{}{extension}", basename.display()))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("the file must exist"))
        .expect("the file must hold JSON")
}

/// The ordinary write asks for the samples and nothing else. The datatype is
/// not among the inputs anywhere: it is a function of the sample type, and the
/// writer is the one place that knows the sample type at construction.
#[test]
fn a_writer_needs_only_the_samples() {
    let dir = TempDir::new().expect("a temp dir");
    let basename = dir.path().join("dsc_watch");

    let samples = dsc_samples();
    let mut writer = RecordingWriter::new(&samples);
    writer.global_mut().sample_rate = Some(32_000.0);
    writer.to_file(&basename).expect("writing must succeed");

    let reopened =
        SigMF::from_file(sibling(&basename, ".sigmf-meta")).expect("the recording must reopen");
    assert_eq!(
        reopened
            .samples::<Complex<f32>>()
            .expect("the samples must read back"),
        samples
    );
    assert_eq!(reopened.metadata.global.sample_rate, Some(32_000.0));
}

/// `global_mut` is an escape hatch to every Global field, and the one field it
/// cannot smuggle into the file is the datatype: the writer derives that from
/// the samples at the moment of writing, which is the crate's central
/// guarantee carried over from the old write path.
#[test]
fn the_datatype_written_describes_the_samples_not_a_claim() {
    let dir = TempDir::new().expect("a temp dir");
    let basename = dir.path().join("dsc_watch");

    let samples = dsc_samples();
    let mut writer = RecordingWriter::new(&samples);
    writer.global_mut().datatype = "ri16_le".parse().expect("a valid datatype");
    writer.to_file(&basename).expect("writing must succeed");

    let written = read_json(&sibling(&basename, ".sigmf-meta"));
    assert_eq!(
        written["global"]["core:datatype"],
        json!("cf32_le"),
        "the file must describe the samples it holds, not the claim it was handed"
    );
}

/// Writing hands back the Recording it wrote, already opened: the corrected
/// metadata for inspection, and the samples readable through it.
#[test]
fn to_file_returns_the_recording_it_wrote() {
    let dir = TempDir::new().expect("a temp dir");
    let basename = dir.path().join("dsc_watch");

    let samples = dsc_samples();
    let written = RecordingWriter::new(&samples)
        .to_file(&basename)
        .expect("writing must succeed");

    assert_eq!(written.metadata.global.datatype.to_string(), "cf32_le");
    assert!(
        written.metadata.global.sha512.is_some(),
        "checksummed by default"
    );
    assert_eq!(
        written
            .samples::<Complex<f32>>()
            .expect("the returned recording must be readable"),
        samples
    );
}

/// The byte-order knob is not a preference the file keeps to itself: choosing
/// big-endian must be stated by the emitted datatype and visible in the bytes.
#[test]
fn the_byte_order_option_is_stated_in_the_datatype() {
    let dir = TempDir::new().expect("a temp dir");
    let basename = dir.path().join("dsc_watch");

    let samples = dsc_samples();
    RecordingWriter::new(&samples)
        .endianness(BigEndian)
        .to_file(&basename)
        .expect("writing must succeed");

    let written = read_json(&sibling(&basename, ".sigmf-meta"));
    assert_eq!(written["global"]["core:datatype"], json!("cf32_be"));

    let data = fs::read(sibling(&basename, ".sigmf-data")).expect("the Dataset was written");
    assert_eq!(
        &data[0..4],
        1.0f32.to_be_bytes(),
        "the first in-phase component must actually be big-endian on disk"
    );
}

/// Turning the checksum off clears `core:sha512` rather than carrying forward
/// a hash of a Dataset that no longer exists.
#[test]
fn checksum_off_clears_a_hash_rather_than_leaving_it_stale() {
    let dir = TempDir::new().expect("a temp dir");
    let basename = dir.path().join("dsc_watch");
    let rewrite = dir.path().join("rewrite");

    let samples = dsc_samples();
    let first = RecordingWriter::new(&samples)
        .to_file(&basename)
        .expect("writing must succeed");
    assert!(first.metadata.global.sha512.is_some(), "on by default");

    RecordingWriter::with_metadata(&samples, first.metadata)
        .checksum(false)
        .to_file(&rewrite)
        .expect("rewriting must succeed");

    let written = read_json(&sibling(&rewrite, ".sigmf-meta"));
    assert!(
        written["global"].get("core:sha512").is_none(),
        "a stale hash is worse than no hash"
    );
}

/// Read → write is a first-class path: a writer seeded with an opened
/// Recording's document reproduces both files, captures and annotations
/// included.
#[test]
fn with_metadata_carries_the_whole_document_through() {
    let dir = TempDir::new().expect("a temp dir");
    let basename = dir.path().join("dsc_watch");
    let copy = dir.path().join("copy");

    let samples = dsc_samples();
    let mut writer = RecordingWriter::new(&samples);
    writer.global_mut().sample_rate = Some(32_000.0);
    writer.global_mut().description = Some("DSC watch on 16 MHz".to_string());
    let mut capture = CaptureMetadata::new(0);
    capture.frequency = Some(16_804_500.0);
    writer.captures_mut().push(capture);
    writer.annotations_mut().push(
        serde_json::from_value(json!({
            "core:sample_start": 0,
            "core:sample_count": 3,
            "core:label": "FSK burst",
        }))
        .expect("the annotation literal must deserialize"),
    );
    writer.to_file(&basename).expect("writing must succeed");

    let reopened =
        SigMF::from_file(sibling(&basename, ".sigmf-meta")).expect("the recording must reopen");
    let samples_back = reopened
        .samples::<Complex<f32>>()
        .expect("the samples must read back");
    RecordingWriter::with_metadata(&samples_back, reopened.metadata)
        .to_file(&copy)
        .expect("the copy must write");

    assert_eq!(
        read_json(&sibling(&basename, ".sigmf-meta")),
        read_json(&sibling(&copy, ".sigmf-meta")),
        "the round-tripped document must survive intact"
    );
    assert_eq!(
        fs::read(sibling(&basename, ".sigmf-data")).expect("the original Dataset"),
        fs::read(sibling(&copy, ".sigmf-data")).expect("the copied Dataset"),
        "and so must the bytes"
    );
}

/// A writer seeded from an opened Recording defaults to the byte order that
/// Recording already has, so a round-trip cannot silently flip it.
#[test]
fn with_metadata_keeps_the_byte_order_it_was_handed() {
    let dir = TempDir::new().expect("a temp dir");
    let basename = dir.path().join("dsc_watch");
    let copy = dir.path().join("copy");

    let samples = dsc_samples();
    let first = RecordingWriter::new(&samples)
        .endianness(BigEndian)
        .to_file(&basename)
        .expect("writing must succeed");

    let samples_back = first
        .samples::<Complex<f32>>()
        .expect("the samples must read back");
    let copied = RecordingWriter::with_metadata(&samples_back, first.metadata)
        .to_file(&copy)
        .expect("the copy must write");

    assert_eq!(
        copied.metadata.global.datatype.to_string(),
        "cf32_be",
        "a round-trip must not silently flip the byte order"
    );
}

/// A flat `&[S]` is one channel by construction, so a Global claiming several
/// channels describes something the writer cannot honestly write.
#[test]
fn a_flat_slice_cannot_claim_channels() {
    let dir = TempDir::new().expect("a temp dir");
    let basename = dir.path().join("dsc_watch");

    let samples = dsc_samples();
    let mut writer = RecordingWriter::new(&samples);
    writer.global_mut().num_channels = Some(2);
    let err = writer
        .to_file(&basename)
        .expect_err("two channels in a flat slice must be refused");
    assert!(matches!(
        err,
        Error::Metadata(MetadataError::MultiChannelDataset(2))
    ));
}

/// The constructor carries the segment's one required field and nothing else:
/// a fresh capture must not invent claims the author never made.
#[test]
fn a_new_capture_carries_only_its_sample_start() {
    let capture = CaptureMetadata::new(5);
    let value = serde_json::to_value(&capture).expect("a capture must serialize");
    assert_eq!(value, json!({"core:sample_start": 5}));
}

/// `describing` is the constructor for bytes this crate will never see — a
/// Non-Conforming Dataset, a `metadata_only` document — where only the author
/// knows the datatype and states it as a value. It fills in the two fields the
/// specification requires and invents nothing else.
#[test]
fn describing_fills_in_what_the_spec_requires_and_nothing_else() {
    let global = GlobalMetadata::describing("ri16_le".parse().expect("a valid datatype"));
    assert_eq!(global.datatype.to_string(), "ri16_le");
    assert_eq!(global.version, SIGMF_VERSION);
    assert_eq!(
        serde_json::to_value(&global).expect("a global must serialize"),
        json!({"core:datatype": "ri16_le", "core:version": SIGMF_VERSION})
    );
}
