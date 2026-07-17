//! Read and write [SigMF](https://sigmf.org) recordings: recorded radio signals, and
//! the metadata that describes them.
//!
//! A **Recording** is two files sharing a basename. `foo.sigmf-data` holds raw
//! interleaved samples and nothing else — no header, no framing. `foo.sigmf-meta` is
//! a JSON sidecar saying what those bytes are: sample format, sample rate, centre
//! frequency, when they were recorded. The point of the format is that the numbers
//! needed to interpret the bytes travel *with* the bytes, instead of living in a
//! README or someone's memory.
//!
//! Start at [`SigMF`]. [`SigMF::to_file`] writes both files of a Recording,
//! [`SigMF::from_file`] opens one, and [`SigMF::samples`] decodes the Dataset.
//! [`Metadata`] is the document itself, should you want to build or inspect one
//! without touching a disk.
//!
//! # Examples
//!
//! Write a Recording and read it back:
//!
//! ```
//! use sigmf::num_complex::Complex;
//! use sigmf::{GlobalMetadata, Metadata, SigMF};
//! # let dir = tempfile::tempdir().expect("a temporary directory");
//! # let basename = dir.path().join("dsc_watch");
//!
//! let mut recording = SigMF::new(Metadata {
//!     global: GlobalMetadata::new("cf32_le".parse().expect("a valid datatype")),
//!     captures: vec![],
//!     annotations: vec![],
//! });
//! recording.metadata.global.sample_rate = Some(32_000.0);
//! recording.metadata.global.recorder = Some("winradio-agent".to_string());
//!
//! // Writes `dsc_watch.sigmf-data` and `dsc_watch.sigmf-meta`.
//! let samples = vec![Complex::new(1.0f32, 0.0), Complex::new(0.0, -1.0)];
//! recording.to_file(&basename, &samples)?;
//!
//! let reopened = SigMF::from_file(dir.path().join("dsc_watch.sigmf-meta"))?;
//! assert_eq!(reopened.metadata.global.sample_rate, Some(32_000.0));
//! assert_eq!(reopened.samples::<Complex<f32>>()?, samples);
//! # Ok::<(), sigmf::Error>(())
//! ```
//!
//! # `core:datatype` is a claim about the bytes
//!
//! `core:datatype` says how to read every byte of the Dataset, and a Dataset is
//! nothing but bytes. Get the field wrong and nothing errors — `cf32_le` bytes read
//! as `ci16_le` yield plausible noise at the wrong scale, and a waterfall of
//! plausible noise looks exactly like a waterfall.
//!
//! So this crate does not accept the claim, it *derives* it. [`SigMF::to_file`] is
//! generic over the sample type, sets `core:datatype` from that type, and overwrites
//! whatever the [`GlobalMetadata`] held: the field and the bytes cannot disagree,
//! because only one of them is an input. [`SigMF::samples`] enforces the same
//! equation from the other side, erroring rather than reinterpreting.
//!
//! What the crate cannot defend is a [`Metadata`] you fill in and serialize yourself.
//! `to_file` cannot lie; [`Metadata::to_json`] will write whatever you put in it.
//!
//! # The specification is the schema
//!
//! This crate implements SigMF **v1.2.6** ([`SIGMF_VERSION`]). The specification is
//! published at [sigmf.org](https://sigmf.org), but there is no specification
//! document in the upstream repository to read: it is *generated* from
//! [`sigmf-schema.json`](https://github.com/sigmf/SigMF/blob/main/sigmf-schema.json),
//! which makes the schema the specification rather than a description of one. This
//! crate vendors it and uses it as a test oracle, validating against it both the
//! fixtures it is judged by and the metadata it writes.

// A library that parses documents from anywhere and indexes into buffers by
// offsets those documents chose is exactly the kind that should not be able to
// reach for `unsafe` -- and this one has never needed it. `forbid` rather than
// `deny` so that the guarantee cannot be turned off locally by the module that
// most wants to.
#![forbid(unsafe_code)]
// This crate's conventions have required documentation on every public item since
// before it was published, and nothing enforced it, so the surface it inherited went
// undocumented while each new change documented itself. `cargo doc` could not help:
// it warns about broken links in the docs that exist and says nothing at all about
// the docs that do not, so a crate with no documentation whatsoever builds clean.
#![deny(missing_docs)]

// Compiles the README's example as a doc-test, so an API change that outdates the
// landing page turns the build red rather than shipping. `cfg(doctest)` keeps it out
// of the rendered docs, where it would only duplicate the crate docs above.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

/// The complex-number type this crate's sample vocabulary is built on.
///
/// Re-exported because it is a *public* dependency: `Sample` is implemented for
/// `num_complex::Complex<f32>` and not for some structurally identical type from
/// another copy of the crate, so a caller whose `num-complex` resolves to a
/// different major than ours would find `to_file` mysteriously unwilling to take
/// their samples. Reaching for `sigmf::num_complex` instead of a direct dependency
/// makes that impossible to get wrong.
pub use num_complex;

/// The extension of a Recording's Metadata file, dot included.
///
/// Public because a caller naming or looking for Recordings needs the same string
/// this crate matches on, and one that hardcodes `".sigmf-meta"` for itself is free
/// to drift from us without either of us noticing.
pub const SIGMF_METADATA_EXT: &str = ".sigmf-meta";

/// The extension of a compliant Recording's Dataset file, dot included.
///
/// A Non-Conforming Dataset is named by `core:dataset` instead and need not use
/// this.
pub const SIGMF_DATASET_EXT: &str = ".sigmf-data";

// `.sigmf` (an Archive) and `.sigmf-collection` (a Collection) sat here too, unread
// by any code path for the crate's whole life. Deleted rather than silenced with an
// `#[allow(dead_code)]`: this crate supports neither format, so exporting the names
// would advertise what it cannot do, and keeping them private kept two string
// literals nothing could reach. They are two lines in the specification, and git
// remembers the spelling.

/// Every public item, which the crate root re-exports.
///
/// Private as of 0.2.0. Until then this module was public and was the only way to
/// name a type in this crate, so every caller wrote its name twice over — and a
/// name repeated is not a namespace, it is a stutter. The module survives its own
/// privacy because deleting it would re-indent sixteen hundred lines and bury a
/// release's worth of real changes in whitespace.
mod sigmf {
    use crate::{SIGMF_DATASET_EXT, SIGMF_METADATA_EXT};
    use core::fmt::Debug;
    use num_complex::Complex;
    use serde_json::Value;
    use sha2::{Digest, Sha512};
    use std::collections::BTreeMap as Map;
    use std::ffi::OsStr;
    use std::fmt::{self, Write as _};
    use std::ops::Range;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use serde::{Deserialize, Serialize};

    /// The version of the SigMF specification this crate implements, in the form
    /// `core:version` takes.
    ///
    /// Kept in step with the schema vendored at `tests/spec/sigmf-schema.json`, and
    /// asserted against it by the test suite so the two cannot drift apart.
    pub const SIGMF_VERSION: &str = "1.2.6";

    /// A Recording: a [`Metadata`] document, and the Dataset it describes.
    ///
    /// The Dataset is referred to by path and read only on demand, so this is cheap
    /// to hold regardless of how many samples it names. [`from_file`](Self::from_file)
    /// opens an existing Recording, [`new`](Self::new) starts one that does not exist
    /// yet, and [`to_file`](Self::to_file) writes both of its files.
    #[derive(Debug)]
    pub struct SigMF {
        /// The document describing the Dataset.
        ///
        /// Public because it is the whole point: reading `core:sample_rate` off a
        /// Recording, or setting `core:recorder` before writing one, is what callers
        /// come here to do. Note that [`to_file`](Self::to_file) *overwrites*
        /// `global.datatype` and `global.sha512` from the samples it is given, so
        /// setting either by hand accomplishes nothing.
        pub metadata: Metadata,

        /// The Dataset this Recording's samples live in, if it has one.
        ///
        /// `None` for a `core:metadata_only` Recording, for a Metadata file whose
        /// name does not yield a sibling, and for one [`new`](Self::new) has built
        /// but nothing has yet written.
        datafile: Option<PathBuf>,
    }

    impl SigMF {
        /// Open a Recording, given the path of its `.sigmf-meta` file.
        ///
        /// The Dataset is not read here, or even opened. Only its name is worked
        /// out: `core:metadata_only` means there is none, `core:dataset` names it
        /// outright, and otherwise it is the sibling `<basename>.sigmf-data`.
        /// Opening the metadata of a hundred-gigabyte Recording therefore costs the
        /// size of its sidecar, and [`samples`](Self::samples) is the call that goes
        /// to disk for the rest.
        ///
        /// # Errors
        ///
        /// [`Error::Io`] if the Metadata file cannot be read, [`Error::Json`] if it
        /// is not a valid SigMF document — which includes a `core:datatype` that
        /// describes no possible bytes — or
        /// [`MetadataError::DatasetPathEscapesDirectory`] if `core:dataset` names
        /// something other than a file beside the Metadata file.
        pub fn from_file<T: AsRef<Path>>(path: T) -> Result<Self, Error> {
            let path = path.as_ref();
            let metadata_file = fs::File::open(path).map_err(at(path))?;
            let metadata: Metadata = serde_json::from_reader(metadata_file)?;
            let datafile = dataset_path(path, &metadata)?;
            Ok(Self { metadata, datafile })
        }

        /// Where each Captures segment's samples sit in the Dataset, as byte ranges.
        ///
        /// The Dataset is measured, not read: this needs its length and nothing
        /// else. See [`Metadata::capture_boundaries`], which does the arithmetic
        /// and documents it.
        ///
        /// # Errors
        ///
        /// Returns [`MetadataError::NoDataset`] if the Recording has no Dataset to
        /// measure, [`Error::Io`] if it cannot be measured, or
        /// [`MetadataError::CaptureOutOfBounds`] if the Metadata describes bytes
        /// the Dataset does not have.
        pub fn capture_boundaries(&self) -> Result<Vec<Range<u64>>, Error> {
            let path = self.datafile.as_ref().ok_or(MetadataError::NoDataset)?;
            let dataset_len = fs::metadata(path).map_err(at(path))?.len();
            Ok(self.metadata.capture_boundaries(dataset_len)?)
        }

        /// Every sample in the Dataset, in order, decoded as `S`.
        ///
        /// # `S` is checked, not assumed
        ///
        /// `core:datatype` must describe `S` exactly, or this errors. That check is
        /// the whole reason the method is generic rather than handing back bytes:
        /// reading a `cf32_le` Dataset as `ci16_le` produces no error and no
        /// obviously wrong number, just plausible noise at the wrong scale — the
        /// same silent-garbage failure that [`to_file`](Self::to_file) prevents on
        /// the way out, arriving from the other direction. Nothing about a `&[u8]`
        /// can be checked; `S` can.
        ///
        /// # Cost
        ///
        /// The whole Dataset is read into memory and decoded. That is inherent in
        /// returning a `Vec<S>` — a caller asking for every sample has asked for
        /// them all — but it is worth knowing before pointing this at a Recording
        /// far larger than RAM. [`capture_boundaries`](Self::capture_boundaries)
        /// answers where the samples are without reading any of them.
        ///
        /// # Errors
        ///
        /// [`MetadataError::DatatypeMismatch`] if `S` is not what `core:datatype`
        /// says, [`MetadataError::MultiChannelDataset`] if the samples are
        /// interleaved across channels and a flat `Vec<S>` would misrepresent them,
        /// [`MetadataError::NoDataset`] if there is no Dataset to read,
        /// [`MetadataError::PartialSample`] if a segment's bytes are not a whole
        /// number of samples, or [`Error::Io`].
        ///
        /// # Examples
        ///
        /// Asking for the wrong type is refused, not reinterpreted. Eight `cf32_le`
        /// bytes are also two perfectly good `ci16_le` samples, and that is exactly
        /// the reading this rules out:
        ///
        /// ```
        /// use sigmf::num_complex::Complex;
        /// use sigmf::{Error, GlobalMetadata, Metadata, MetadataError, SigMF};
        /// # let dir = tempfile::tempdir().expect("a temporary directory");
        /// # let basename = dir.path().join("capture");
        ///
        /// let mut recording = SigMF::new(Metadata {
        ///     global: GlobalMetadata::new("cf32_le".parse().expect("a valid datatype")),
        ///     captures: vec![],
        ///     annotations: vec![],
        /// });
        /// recording.to_file(&basename, &[Complex::new(1.0f32, 0.0)])?;
        ///
        /// let reopened = SigMF::from_file(dir.path().join("capture.sigmf-meta"))?;
        /// assert_eq!(reopened.samples::<Complex<f32>>()?, [Complex::new(1.0f32, 0.0)]);
        ///
        /// let err = reopened
        ///     .samples::<Complex<i16>>()
        ///     .expect_err("the Dataset is cf32_le, and says so");
        /// assert!(matches!(
        ///     err,
        ///     Error::Metadata(MetadataError::DatatypeMismatch { .. })
        /// ));
        /// # Ok::<(), sigmf::Error>(())
        /// ```
        pub fn samples<S: Sample>(&self) -> Result<Vec<S>, Error> {
            let datatype = self.metadata.global.datatype;

            // A one-byte component has no byte order, so for `ri8`/`ru8` the
            // argument here cannot change the answer; for every other type the
            // stored order is the only one that could possibly match.
            let endianness = datatype.endianness().unwrap_or(Endianness::LittleEndian);
            let requested = DataFormat::of::<S>(endianness);
            if requested != datatype {
                return Err(MetadataError::DatatypeMismatch {
                    stored: datatype,
                    requested,
                }
                .into());
            }

            // The mirror of `to_file_with`'s refusal, and for the same reason: with
            // several channels interleaved into the Dataset, one element of a
            // `Vec<S>` is one channel's sample, and the Vec says nothing about
            // which. Deinterleaving wants a return type that admits channels exist.
            if let Some(channels) = self.metadata.global.num_channels {
                if channels != 1 {
                    return Err(MetadataError::MultiChannelDataset(channels).into());
                }
            }

            let path = self.datafile.as_ref().ok_or(MetadataError::NoDataset)?;
            let data = fs::read(path).map_err(at(path))?;
            let boundaries = self.metadata.capture_boundaries(data.len() as u64)?;

            let sample_size = datatype.size() as usize;
            let mut samples = Vec::new();
            for range in boundaries {
                // `capture_boundaries` has already established that this range lies
                // within the Dataset, which is what makes both the cast and the
                // index safe.
                let bytes = &data[range.start as usize..range.end as usize];
                let whole_samples = bytes.chunks_exact(sample_size);
                if !whole_samples.remainder().is_empty() {
                    return Err(MetadataError::PartialSample {
                        bytes: bytes.len() as u64,
                        datatype,
                    }
                    .into());
                }
                samples.extend(whole_samples.map(|sample| S::decode(endianness, sample)));
            }
            Ok(samples)
        }

        /// A Recording that describes samples not yet written.
        ///
        /// `metadata.global.datatype` is not read by [`to_file`](Self::to_file) —
        /// it is overwritten by it — so whatever [`GlobalMetadata::new`] was handed
        /// is a placeholder until the samples arrive and settle the question.
        pub fn new(metadata: Metadata) -> Self {
            Self {
                metadata,
                datafile: None,
            }
        }

        /// Write both files of the Recording, little-endian, with a checksum.
        ///
        /// See [`to_file_with`](Self::to_file_with), which this defers to, for what
        /// gets written and what gets overwritten.
        ///
        /// # Examples
        ///
        /// A Global's `core:datatype` is a claim, and writing settles it. Here the
        /// claim is wrong, and the file describes its own bytes anyway:
        ///
        /// ```
        /// use sigmf::num_complex::Complex;
        /// use sigmf::{GlobalMetadata, Metadata, SigMF};
        /// # let dir = tempfile::tempdir().expect("a temporary directory");
        /// # let basename = dir.path().join("capture");
        ///
        /// // A Global claiming 16-bit real samples ...
        /// let mut recording = SigMF::new(Metadata {
        ///     global: GlobalMetadata::new("ri16_le".parse().expect("a valid datatype")),
        ///     captures: vec![],
        ///     annotations: vec![],
        /// });
        ///
        /// // ... handed complex 32-bit floats.
        /// recording.to_file(&basename, &[Complex::new(1.0f32, 0.0)])?;
        ///
        /// // The samples win: `ri16_le` was never written anywhere.
        /// assert_eq!(recording.metadata.global.datatype.to_string(), "cf32_le");
        ///
        /// // Eight bytes for the one sample, and a checksum over them.
        /// let data = std::fs::metadata(dir.path().join("capture.sigmf-data"))
        ///     .expect("the Dataset was written");
        /// assert_eq!(data.len(), 8);
        /// assert!(recording.metadata.global.sha512.is_some());
        /// # Ok::<(), sigmf::Error>(())
        /// ```
        pub fn to_file<S: Sample, P: AsRef<Path>>(
            &mut self,
            basename: P,
            samples: &[S],
        ) -> Result<(), Error> {
            self.to_file_with(basename, samples, WriteOptions::default())
        }

        /// Write both files of the Recording: `basename.sigmf-data` from `samples`,
        /// and `basename.sigmf-meta` describing them.
        ///
        /// # `core:datatype` is set, not read
        ///
        /// The datatype is derived from `S` and **overwrites** whatever
        /// `self.metadata.global.datatype` held. This is the crate's central
        /// guarantee and the reason the write path is generic: the field and the
        /// bytes cannot disagree, because only one of them is an input. A caller
        /// who builds a Global saying `ri16_le` and then writes `Complex<f32>`
        /// samples gets a `cf32_le` file — their claim was wrong, and the file
        /// tells the truth about its own contents.
        ///
        /// `core:sha512` is set the same way and for the same reason: computed from
        /// the bytes being written, or, if [`WriteOptions::checksum`] is off,
        /// *cleared* rather than left to describe a Dataset that no longer exists.
        ///
        /// # Ordering
        ///
        /// The Dataset is written first and the Metadata last, so that a process
        /// that dies mid-write leaves either a complete Recording or a `.sigmf-data`
        /// with no sidecar — visibly unfinished. The reverse order can leave a
        /// sidecar that looks valid while describing a truncated Dataset, which is
        /// worse than an obvious failure. With `core:sha512` written, the
        /// distinction is not merely visible but provable.
        ///
        /// # Errors
        ///
        /// Returns [`MetadataError::MultiChannelDataset`] if `core:num_channels` is
        /// set to anything but 1 (see the SigMF specification's advice to use
        /// Collections instead), or [`Error::Io`] if either file cannot be written.
        ///
        /// # Examples
        ///
        /// Writing big-endian and without a checksum. Note that turning the checksum
        /// off *clears* `core:sha512`, so a Recording written twice cannot end up
        /// carrying a hash of the Dataset it used to have:
        ///
        /// ```
        /// use sigmf::num_complex::Complex;
        /// use sigmf::{Endianness, GlobalMetadata, Metadata, SigMF, WriteOptions};
        /// # let dir = tempfile::tempdir().expect("a temporary directory");
        /// # let basename = dir.path().join("capture");
        ///
        /// let mut recording = SigMF::new(Metadata {
        ///     global: GlobalMetadata::new("cf32_le".parse().expect("a valid datatype")),
        ///     captures: vec![],
        ///     annotations: vec![],
        /// });
        ///
        /// recording.to_file(&basename, &[Complex::new(1.0f32, 0.0)])?;
        /// assert!(recording.metadata.global.sha512.is_some(), "on by default");
        ///
        /// recording.to_file_with(
        ///     &basename,
        ///     &[Complex::new(1.0f32, 0.0)],
        ///     WriteOptions::default()
        ///         .endianness(Endianness::BigEndian)
        ///         .checksum(false),
        /// )?;
        ///
        /// // The byte order is not a preference the file keeps to itself.
        /// assert_eq!(recording.metadata.global.datatype.to_string(), "cf32_be");
        /// assert!(recording.metadata.global.sha512.is_none(), "cleared, not stale");
        /// # Ok::<(), sigmf::Error>(())
        /// ```
        pub fn to_file_with<S: Sample, P: AsRef<Path>>(
            &mut self,
            basename: P,
            samples: &[S],
            options: WriteOptions,
        ) -> Result<(), Error> {
            // A `&[S]` is one channel by construction: nothing in the slice can say
            // where one channel ends and the next begins, so honouring
            // `core:num_channels > 1` would mean writing a datatype that describes
            // something other than the bytes.
            if let Some(channels) = self.metadata.global.num_channels {
                if channels != 1 {
                    return Err(MetadataError::MultiChannelDataset(channels).into());
                }
            }

            let datatype = DataFormat::of::<S>(options.endianness);
            let mut data = Vec::with_capacity(samples.len() * datatype.size() as usize);
            for sample in samples {
                sample.encode(options.endianness, &mut data);
            }

            self.metadata.global.datatype = datatype;
            self.metadata.global.sha512 =
                options.checksum.then(|| hex_encode(&Sha512::digest(&data)));

            let data_path = append_extension(basename.as_ref(), SIGMF_DATASET_EXT);
            let metadata_path = append_extension(basename.as_ref(), SIGMF_METADATA_EXT);

            fs::write(&data_path, &data).map_err(at(&data_path))?;
            fs::write(&metadata_path, self.metadata.to_json()?).map_err(at(&metadata_path))?;

            self.datafile = Some(data_path);
            Ok(())
        }
    }

    /// The Dataset file that belongs to a Metadata file at `metadata_path`.
    ///
    /// `None` means the Recording has no Dataset to point at, which the
    /// specification allows in two ways: `core:metadata_only`, which says the
    /// Dataset was deliberately not distributed; and a Metadata file not named
    /// `<basename>.sigmf-meta`, which is not a compliant Recording's Metadata file
    /// and so has no derivable sibling.
    ///
    /// Otherwise the specification names the file twice over, and this follows both
    /// clauses. With `core:dataset`, that field *is* the filename — the Recording is
    /// a Non-Conforming Dataset and the name is arbitrary. Without it: "it MUST have
    /// a compliant SigMF Dataset ... which MUST use the same base filename as the
    /// Metadata file and use the `.sigmf-data` extension".
    ///
    /// Note that `Path::set_extension` is the right tool for that second clause and
    /// the wrong one for [`append_extension`]'s job, which looks like the same job.
    /// It replaces the last dotted segment — and here the last dotted segment is
    /// `.sigmf-meta`, which this function has just matched, so `dsc_16804.5kHz`
    /// keeps its `.5kHz`. Given a bare basename there is no such guarantee, and
    /// that is precisely the case [`append_extension`] exists for.
    fn dataset_path(
        metadata_path: &Path,
        metadata: &Metadata,
    ) -> Result<Option<PathBuf>, MetadataError> {
        if metadata.global.metadata_only == Some(true) {
            return Ok(None);
        }

        let Some(dataset) = &metadata.global.dataset else {
            if metadata_path.extension() != Some(OsStr::new(meta_ext())) {
                return Ok(None);
            }
            return Ok(Some(metadata_path.with_extension(data_ext())));
        };

        // "note that this string only includes the filename, not directory". A value
        // that disagrees is not just non-conformant: it is a Metadata file, possibly
        // fetched from anywhere, directing a reader at a path of its choosing. The
        // spec's own rule is the whole defence, so enforce it rather than resolving
        // whatever arrives and hoping it stayed put.
        let name = Path::new(dataset);
        if name.file_name() != Some(name.as_os_str()) {
            return Err(MetadataError::DatasetPathEscapesDirectory(dataset.clone()));
        }
        Ok(Some(metadata_path.with_file_name(name)))
    }

    /// `.sigmf-meta` without the dot, which is what [`Path::extension`] deals in.
    fn meta_ext() -> &'static str {
        SIGMF_METADATA_EXT.trim_start_matches('.')
    }

    /// `.sigmf-data` without the dot, which is what [`Path::set_extension`] deals in.
    fn data_ext() -> &'static str {
        SIGMF_DATASET_EXT.trim_start_matches('.')
    }

    /// A Recording's two files share a basename with an extension **appended**.
    ///
    /// Appended, not substituted: `Path::set_extension` would turn the perfectly
    /// good basename `dsc_2026-07-16T09.14` into `dsc_2026-07-16T09.sigmf-data`,
    /// silently, by treating the last dotted segment as an extension to replace.
    fn append_extension(basename: &Path, extension: &str) -> PathBuf {
        let mut name = basename.as_os_str().to_owned();
        name.push(extension);
        PathBuf::from(name)
    }

    fn hex_encode(bytes: &[u8]) -> String {
        let mut hex = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            // Writing to a String is infallible.
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }

    /// Names the file an [`std::io::Error`] happened to, on its way into [`Error`].
    ///
    /// Spelled to be used as `fs::read(path).map_err(at(path))?`. A bare
    /// `std::io::Error` carries no path — "No such file or directory" is the whole
    /// message — and a Recording is two files, so an error without one leaves the
    /// caller unable to tell which half is missing. Taking the path as an argument
    /// rather than deriving it from a `#[from]` is what makes that impossible to
    /// forget: there is no conversion for `?` to reach for.
    fn at(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
        move |source| Error::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    /// What a JSON value is, in the specification's own vocabulary, for an error
    /// message that has to tell a caller what they handed over.
    fn json_type_name(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    /// The knobs [`SigMF::to_file_with`] turns, with sane values from [`Default`].
    ///
    /// Both defaults are safe to take blind, and it is worth saying why, because
    /// defaulting a field of `core:datatype` would be alarming in any other design:
    /// whatever byte order this picks is the byte order the emitted datatype
    /// *states*. The choice cannot make a Recording lie about itself; at worst it
    /// makes one inconvenient to a reader that wanted the other order.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct WriteOptions {
        endianness: Endianness,
        checksum: bool,
    }

    impl Default for WriteOptions {
        /// Little-endian, checksummed.
        ///
        /// Little-endian because it is what essentially every IQ recorder emits and
        /// what the schema's own `core:datatype` examples use. Checksummed because
        /// a Recording that cannot prove its Dataset is intact is one you can only
        /// hope about — and the hash costs one pass over a buffer already in hand.
        fn default() -> Self {
            Self {
                endianness: Endianness::LittleEndian,
                checksum: true,
            }
        }
    }

    impl WriteOptions {
        /// Write samples in this byte order, and say so in `core:datatype`.
        pub fn endianness(mut self, endianness: Endianness) -> Self {
            self.endianness = endianness;
            self
        }

        /// Whether to compute `core:sha512` over the Dataset.
        ///
        /// Turning this off omits the field rather than preserving any hash the
        /// Global already carried, which would otherwise describe a Dataset that
        /// has just been replaced.
        pub fn checksum(mut self, checksum: bool) -> Self {
            self.checksum = checksum;
            self
        }
    }

    /// A `.sigmf-meta` document: everything known about a Dataset except its bytes.
    ///
    /// The specification gives this three top-level scopes, and they answer different
    /// questions. [`global`](Self::global) describes the Recording as a whole — what
    /// the samples *are*. [`captures`](Self::captures) describes the Dataset in
    /// segments, each taking effect at a sample index — what the receiver was *doing*
    /// while it recorded them. [`annotations`](Self::annotations) describes things
    /// found *in* the samples, at a sample index and optionally a frequency band.
    ///
    /// A Metadata can be manipulated without any Dataset present, which is what makes
    /// [`from_json`](Self::from_json) and [`to_json`](Self::to_json) worth having
    /// alongside [`SigMF`]. It is also the type that can lie: a Global's
    /// `core:datatype` is checked against reality only by [`SigMF::to_file`] and
    /// [`SigMF::samples`], never by this type on its own.
    #[derive(Debug, Deserialize, Serialize)]
    pub struct Metadata {
        /// What the samples are: format, rate, and provenance.
        pub global: GlobalMetadata,

        /// The Dataset in segments, each taking effect at a sample index.
        ///
        /// A Recording that never retunes has one segment; one that hops has a
        /// segment per hop. Ordering is the specification's, not this crate's: the
        /// array SHOULD be sorted by `core:sample_start`, and
        /// [`capture_boundaries`](Self::capture_boundaries) reads it in the order it
        /// finds it.
        pub captures: Vec<CaptureMetadata>,

        /// Features found in the samples, each at a sample index and optionally a
        /// frequency band.
        ///
        /// Nothing in the format requires these to be present, correct, or produced
        /// by whoever made the Recording — an annotation is somebody's claim about a
        /// signal, and [`generator`](AnnotationMetadata::generator) records whose.
        pub annotations: Vec<AnnotationMetadata>,
    }

    impl Metadata {
        /// Parse a `.sigmf-meta` document.
        ///
        /// Nothing here reads the Dataset. This function used to take its bytes so
        /// that it could compute capture boundaries on the way past; that is now
        /// [`capture_boundaries`](Self::capture_boundaries)' job, which asks only
        /// for a length.
        ///
        /// # Errors
        ///
        /// Every way a document can be rejected — malformed JSON, a missing
        /// `core:datatype`, a `core:datatype` that describes no possible bytes —
        /// surfaces as a [`serde_json::Error`], because deserialization is where all
        /// of them are caught. The error type says so rather than widening to
        /// [`Error`], which would offer the caller an [`Error::Io`] this function
        /// cannot produce. It converts into [`Error`] for callers who want the one
        /// type.
        pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
            serde_json::from_str(s)
        }

        /// Serialize to a `.sigmf-meta` document, pretty-printed.
        ///
        /// The inverse of [`from_json`](Self::from_json), and named to say so. This
        /// was `to_str`, which the Rust API guidelines spend on a *cheap, borrowing*
        /// conversion — [`std::ffi::OsStr::to_str`] hands back a `&str` and never
        /// allocates. This allocates a whole document, and the guidelines' name for
        /// that is `to_`-something-that-says-what.
        pub fn to_json(&self) -> Result<String, serde_json::Error> {
            serde_json::to_string_pretty(self)
        }

        /// Where each Captures segment's samples sit in a Dataset `dataset_len`
        /// bytes long, as byte ranges.
        ///
        /// # Why a length, and not the Dataset
        ///
        /// A segment's end is defined relative to the Dataset, and the Metadata file
        /// does not record how big the Dataset is — so the boundaries are a function
        /// of both, and this signature says so. It asks for the length because the
        /// length is all it reads: taking the bytes instead would mean a
        /// hundred-gigabyte Recording needs a hundred gigabytes of memory to learn a
        /// number `fs::metadata(path)?.len()` answers for free. Deriving this on a
        /// method rather than storing it on [`CaptureMetadata`] is the same point
        /// made in the type system: the Dataset's length cannot be deserialized out
        /// of a `.sigmf-meta`, so a field claiming to hold the answer can only ever
        /// have been given a default one.
        ///
        /// # Empty `captures`
        ///
        /// An empty Captures array does not mean the Dataset has no samples. The
        /// specification: `"captures": []` implies `"captures": [{"core:sample_start":
        /// 0}]` — one implicit segment covering everything — and that is what this
        /// returns.
        ///
        /// # Errors
        ///
        /// [`MetadataError::CaptureOutOfBounds`] if a segment describes bytes the
        /// Dataset does not have, which includes the case of segments that are not
        /// sorted by `core:sample_start` as the specification requires.
        ///
        /// # Examples
        ///
        /// A Recording that retuned once, halfway through 32 bytes of `cf32_le` — so
        /// four samples of eight bytes, two per segment:
        ///
        /// ```
        /// use sigmf::Metadata;
        ///
        /// let metadata: Metadata = serde_json::from_str(r#"{
        ///     "global": { "core:datatype": "cf32_le", "core:version": "1.2.6" },
        ///     "captures": [
        ///         { "core:sample_start": 0, "core:frequency": 2187500.0 },
        ///         { "core:sample_start": 2, "core:frequency": 8414500.0 }
        ///     ],
        ///     "annotations": []
        /// }"#)?;
        ///
        /// assert_eq!(metadata.capture_boundaries(32)?, [0..16, 16..32]);
        /// # Ok::<(), Box<dyn std::error::Error>>(())
        /// ```
        pub fn capture_boundaries(
            &self,
            dataset_len: u64,
        ) -> Result<Vec<Range<u64>>, MetadataError> {
            let trailing = self.global.trailing_bytes.unwrap_or(0);
            let last_sample_byte = dataset_len.checked_sub(trailing).ok_or(
                MetadataError::TrailingBytesExceedDataset {
                    trailing,
                    dataset_len,
                },
            )?;

            if self.captures.is_empty() {
                // Spelled `once` rather than `vec![0..last_sample_byte]`, which reads
                // ambiguously — one Range to us, and the numbers 0 to n to a reader
                // who has met `vec![0; n]` more recently. Clippy says the same.
                return Ok(std::iter::once(0..last_sample_byte).collect());
            }

            // Every segment of a Recording shares one sample format: the global
            // `core:datatype`, already parsed at the file boundary.
            let sample_size = self.global.datatype.size();

            let mut boundaries = Vec::with_capacity(self.captures.len());
            // `core:sample_start` counts samples, so it cannot see the header bytes
            // that Non-Conforming Datasets put in front of each segment. Their
            // running total is the only thing that accumulates down this loop: every
            // other term is absolute.
            let mut headers = 0u64;
            for (index, capture) in self.captures.iter().enumerate() {
                headers += capture.header_bytes.unwrap_or(0);

                // Checked, not wrapping: `core:sample_start` is a `u64` from a file,
                // and a release build would otherwise answer a sample index near
                // `u64::MAX` with a small, plausible, wrong byte offset.
                let byte_of = |sample_start: u64| -> Result<u64, MetadataError> {
                    sample_size
                        .checked_mul(sample_start)
                        .and_then(|offset| offset.checked_add(headers))
                        .ok_or(MetadataError::SampleStartOutOfRange {
                            index,
                            sample_start,
                        })
                };

                let start = byte_of(capture.sample_start)?;
                let end = match self.captures.get(index + 1) {
                    // This segment runs until the next one's samples begin — minus
                    // that segment's own header, which `byte_of` has not counted yet.
                    Some(next) => byte_of(next.sample_start)?,
                    None => last_sample_byte,
                };

                if start > end || end > last_sample_byte {
                    return Err(MetadataError::CaptureOutOfBounds {
                        index,
                        start,
                        end,
                        dataset_len,
                    });
                }
                boundaries.push(start..end);
            }
            Ok(boundaries)
        }
    }

    /// The `global` scope: what the samples are, and where they came from.
    ///
    /// Only [`datatype`](Self::datatype) and [`version`](Self::version) are required,
    /// which is why [`new`](Self::new) takes one argument and supplies the other.
    /// Everything else is optional in the specification and so `Option` here, with
    /// one exception: [`other`](Self::other) is the catch-all that makes this type
    /// lossless, because the schema does not close this object and extension
    /// namespaces live in it.
    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct GlobalMetadata {
        /// How to read every byte of the Dataset.
        ///
        /// **Do not set this to describe samples you are about to write.**
        /// [`SigMF::to_file`] derives it from the sample type and overwrites this;
        /// the crate-level docs explain why that is not a courtesy but the crate's
        /// central guarantee. Setting it matters only for a Recording you are
        /// serializing by hand, which is the one case nothing can check.
        #[serde(rename = "core:datatype")]
        pub datatype: DataFormat,

        /// Samples per second, in Hz.
        ///
        /// Optional in the specification, and a Recording without it is very nearly
        /// useless — nothing downstream can convert a sample index to a time, or a
        /// bin to a frequency. Set it.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:sample_rate")]
        pub sample_rate: Option<f64>,

        /// The version of the SigMF specification this document is written to, as
        /// `X.Y.Z`.
        ///
        /// [`new`](Self::new) sets it to [`SIGMF_VERSION`]. Reading a Recording does
        /// not check it: this crate parses what it understands and preserves the rest
        /// through [`other`](Self::other), which degrades more gracefully than
        /// refusing a document over a version number.
        #[serde(rename = "core:version")]
        pub version: String,

        /// The number of channels interleaved into the Dataset.
        ///
        /// Absent means 1. Anything other than 1 is refused by both
        /// [`SigMF::to_file`] and [`SigMF::samples`], which deal in a flat `[S]` that
        /// cannot say which channel a sample belongs to — the specification's own
        /// advice is to use a Collection rather than this field.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:num_channels")]
        pub num_channels: Option<u64>,

        /// SHA-512 of the Dataset file, lowercase hex.
        ///
        /// [`SigMF::to_file`] computes and overwrites this, or clears it when
        /// [`WriteOptions::checksum`] is off — a stale hash describing a Dataset that
        /// no longer exists is worse than no hash. Nothing in this crate verifies it
        /// on read.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:sha512")]
        pub sha512: Option<String>,

        /// The sample index of the Dataset's first sample.
        ///
        /// Absent means 0. Non-zero says this Recording is one piece of a stream
        /// split across files: sample indices in SigMF are absolute, so every other
        /// index in this document should be at or above it.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:offset")]
        pub offset: Option<u64>,

        /// A human-readable description of the Recording.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:description")]
        pub description: Option<String>,

        /// Who made the Recording — free text, and the specification suggests it may
        /// include a name, handle, email, or callsign.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:author")]
        pub author: Option<String>,

        /// The DOI (ISO 26324) registered for this Metadata file.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:meta_doi")]
        pub meta_doi: Option<String>,

        /// The DOI (ISO 26324) registered for this Recording's Dataset file.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:data_doi")]
        pub data_doi: Option<String>,

        /// The software that made this Recording.
        ///
        /// The counterpart to [`hw`](Self::hw), and the field that answers "what
        /// wrote this?" six months later. Note that it names the *recorder*, not this
        /// crate — nothing here fills it in for you.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:recorder")]
        pub recorder: Option<String>,

        /// A URL (RFC 3986) for the license the Recording is offered under.
        ///
        /// A URL, not an SPDX identifier or licence text — a bare `MIT` here is not
        /// what the specification asks for.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:license")]
        pub license: Option<String>,

        /// A human-readable description of the hardware used to make the Recording.
        ///
        /// Free text: receiver, antenna, preamp, whatever the next person needs to
        /// interpret what they are looking at. Structured antenna facts have a
        /// dedicated home in the `antenna` extension — see [`AntennaGlobal`].
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:hw")]
        pub hw: Option<String>,

        /// The location of the recording system.
        ///
        /// The Captures scope ([`CaptureMetadata::geolocation`]) is preferred; the
        /// schema keeps this one for backwards compatibility and notes that fixed
        /// recording systems may still use it, so a reader should check this when
        /// the Captures field is absent.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:geolocation")]
        pub geolocation: Option<Geolocation>,

        /// The SigMF extension namespaces this Recording uses.
        ///
        /// Maintained by [`set_extension`](Self::set_extension) and
        /// [`delete_extension`](Self::delete_extension); a reader consults it to
        /// learn which namespaces it must understand before parsing.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:extensions")]
        pub extensions: Option<Vec<Extension>>,

        /// The base name of the `.sigmf-collection` this Recording belongs to, if
        /// it is part of a Collection.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:collection")]
        pub collection: Option<String>,

        /// `true` if this Metadata is distributed deliberately without its Dataset.
        ///
        /// Makes [`SigMF::samples`] and [`SigMF::capture_boundaries`] fail with
        /// [`MetadataError::NoDataset`], which is the point: there is nothing to
        /// read, and the document says so rather than leaving a reader to discover a
        /// missing file.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:metadata_only")]
        pub metadata_only: Option<bool>,

        /// The filename of a Non-Conforming Dataset, extension included.
        ///
        /// Present only for a Recording whose samples live in a file this format did
        /// not produce — a `.wav`, a vendor capture — which is what "Non-Conforming"
        /// means. It names a file beside the Metadata file; a path with a directory
        /// component in it is refused by [`SigMF::from_file`]. Absent, the Dataset is
        /// the sibling `<basename>.sigmf-data`.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:dataset")]
        pub dataset: Option<String>,

        /// Bytes to ignore at the *end* of a Non-Conforming Dataset.
        ///
        /// The footer of a container this format did not write.
        /// [`Metadata::capture_boundaries`] subtracts these, and errors if there are
        /// more of them than the Dataset has bytes. Its counterpart at the other end
        /// is per-segment: [`CaptureMetadata::header_bytes`].
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:trailing_bytes")]
        pub trailing_bytes: Option<u64>,

        /// Every key in the Global object that the fields above do not model.
        ///
        /// This is where extension data such as `antenna:model` lives, and what
        /// [`get_extension`](Self::get_extension) and
        /// [`set_extension`](Self::set_extension) read and write through.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// One Captures segment: what the receiver was doing, from a sample index on.
    ///
    /// A segment takes effect at [`sample_start`](Self::sample_start) and runs until
    /// the next one begins, or to the end of the Dataset. A Recording that sits on
    /// one frequency has a single segment; one that retunes or hops has a segment per
    /// change, and that is how a reader learns the centre frequency of any given
    /// sample. [`Metadata::capture_boundaries`] turns the array into byte ranges.
    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct CaptureMetadata {
        /// The sample index at which this segment takes effect.
        ///
        /// Absolute, and so measured from [`GlobalMetadata::offset`] rather than from
        /// the start of this Dataset — the two differ for a stream split across
        /// files.
        #[serde(rename = "core:sample_start")]
        pub sample_start: u64,

        /// The index of [`sample_start`](Self::sample_start) in the original stream,
        /// if the Dataset holds only part of one.
        ///
        /// Absent means it is the same as `sample_start`. Present, it says samples
        /// were dropped or never captured before this point — a gap this Recording
        /// cannot otherwise express.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:global_index")]
        pub global_index: Option<u64>,

        /// The centre frequency of the signal in this segment, in Hz.
        ///
        /// What the samples are centred *on*, not what is in them: baseband DC in the
        /// Dataset is this frequency on the air.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:frequency")]
        pub frequency: Option<f64>,

        /// When [`sample_start`](Self::sample_start) was recorded.
        ///
        /// The specification requires an RFC 3339 timestamp whose only permitted
        /// offset is `Z` — so `2026-07-17T09:33:00Z`, and not a local time with an
        /// offset. This crate carries the field as an unvalidated `String` and
        /// enforces none of that: it neither parses what it reads nor checks what it
        /// writes, so a value here is only as well-formed as whoever set it.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:datetime")]
        pub datetime: Option<String>,

        /// The location of the recording system at the start of this segment.
        ///
        /// The schema states this is the preferred home for a position, in
        /// preference to [`GlobalMetadata::geolocation`], because it can track a
        /// moving receiver across segments.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:geolocation")]
        pub geolocation: Option<Geolocation>,

        /// Bytes to skip at the start of this segment, for a Non-Conforming Dataset.
        ///
        /// The header of a container this format did not write, sitting physically
        /// where this segment's samples would otherwise begin.
        /// [`Metadata::capture_boundaries`] skips them. Its counterpart at the other
        /// end of the file is [`GlobalMetadata::trailing_bytes`].
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:header_bytes")]
        pub header_bytes: Option<u64>,

        /// Every key in this Captures segment that the fields above do not model.
        ///
        /// The schema sets `additionalProperties: true` on a Captures segment, so
        /// keys outside `core:` are not merely tolerated here — they are the whole
        /// mechanism by which extension namespaces attach per-segment data, and
        /// `antenna:azimuth_angle` on a rotating antenna is the ordinary case, not
        /// an exotic one. Without this field such a key is read into nothing and
        /// written back as nothing.
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// One annotation: something somebody claims is in the samples.
    ///
    /// An annotation marks a feature in a region of the Recording — a burst, a
    /// carrier, a decoded message — bounded in time by
    /// [`sample_start`](Self::sample_start) and [`sample_count`](Self::sample_count),
    /// and optionally in frequency by the two edge fields.
    ///
    /// Nothing here is authoritative. Annotations may be added by anyone at any time,
    /// need not agree with each other, and are not checked against the samples by
    /// this crate or by the format; [`generator`](Self::generator) exists precisely
    /// because a claim is worth only as much as its source.
    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct AnnotationMetadata {
        /// The sample index where the annotated feature begins.
        #[serde(rename = "core:sample_start")]
        pub sample_start: u64,

        /// How many samples the feature covers.
        ///
        /// Absent means the annotation applies from `sample_start` to the end of the
        /// Recording.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:sample_count")]
        pub sample_count: Option<u64>,

        /// The lower edge of the annotated band, in Hz.
        ///
        /// The specification asks for RF rather than baseband where the RF frequency
        /// is known — so an absolute frequency on the air, not an offset from the
        /// segment's [`CaptureMetadata::frequency`]. Paired with
        /// [`freq_upper_edge`](Self::freq_upper_edge): the schema requires either
        /// both or neither.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:freq_lower_edge")]
        pub freq_lower_edge: Option<f64>,

        /// The upper edge of the annotated band, in Hz. See
        /// [`freq_lower_edge`](Self::freq_lower_edge).
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:freq_upper_edge")]
        pub freq_upper_edge: Option<f64>,

        /// A short label for the feature, for a human or a machine.
        ///
        /// The specification recommends keeping it under about 20 characters, since
        /// a common use is a caption on a spectrogram. Longer prose belongs in
        /// [`comment`](Self::comment).
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:label")]
        pub label: Option<String>,

        /// What produced this annotation — a person, a detector, a decoder.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:generator")]
        pub generator: Option<String>,

        /// A longer human-readable comment. See [`label`](Self::label) for the short
        /// form.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:comment")]
        pub comment: Option<String>,

        /// An RFC 4122 UUID for this annotation.
        ///
        /// Carried as an unvalidated `String`; this crate does not check the form.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:uuid")]
        pub uuid: Option<String>,

        /// Every key in this annotation that the fields above do not model.
        ///
        /// The schema sets `additionalProperties: true` on an annotation. This is
        /// the scope where the catch-all matters most: an annotation exists to say
        /// something about a span of samples that `core:` has no vocabulary for, so
        /// the interesting content is *expected* to live under an extension
        /// namespace. See [`CaptureMetadata::other`].
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    /// The location of a recording system: the value of `core:geolocation`.
    ///
    /// The specification requires a single [RFC 7946] GeoJSON Point here, and
    /// states that the Captures scope ([`CaptureMetadata::geolocation`]) is the
    /// preferred home for it, with the Global scope
    /// ([`GlobalMetadata::geolocation`]) kept for backwards compatibility and for
    /// fixed receiving sites.
    ///
    /// GeoJSON writes a position as a bare array whose meaning is purely
    /// positional — `[longitude, latitude]`, never the reverse. That is the
    /// opposite order to how coordinates are usually spoken and written, and a
    /// transposed pair does not fail: it is a valid position somewhere else on
    /// Earth. The components are named here so that the caller states the order
    /// rather than remembers it; the array is assembled on the way out.
    ///
    /// # Examples
    ///
    /// ```
    /// use sigmf::Geolocation;
    ///
    /// // Walvis Bay: 14.5° east, 22.96° south, 7 m above the ellipsoid.
    /// let heard_at = Geolocation {
    ///     altitude: Some(7.0),
    ///     ..Geolocation::new(14.5053, -22.9576)
    /// };
    ///
    /// let json = serde_json::to_string(&heard_at).unwrap();
    /// assert_eq!(
    ///     json,
    ///     r#"{"type":"Point","coordinates":[14.5053,-22.9576,7.0]}"#
    /// );
    /// ```
    ///
    /// [RFC 7946]: https://www.rfc-editor.org/rfc/rfc7946
    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    #[serde(try_from = "GeolocationWire", into = "GeolocationWire")]
    pub struct Geolocation {
        /// Degrees east of the prime meridian, WGS84. The **first** element of the
        /// GeoJSON coordinate array.
        pub longitude: f64,

        /// Degrees north of the equator, WGS84. The **second** element.
        pub latitude: f64,

        /// Metres above the WGS84 ellipsoid — which is neither height above sea
        /// level nor height above ground. The optional third element.
        pub altitude: Option<f64>,

        /// The GeoJSON bounding box, if the writer supplied one.
        ///
        /// Degenerate for a Point and rarely present, but a standard GeoJSON
        /// member rather than a foreign one, so it is modelled rather than left to
        /// [`other`](Self::other). Left as a bare `Vec` because that is all the
        /// schema constrains it to: at least four numbers.
        pub bbox: Option<Vec<f64>>,

        /// GeoJSON *Foreign Members* — any other key on the Point object.
        ///
        /// RFC 7946 section 6.1 permits these, and the specification explicitly
        /// invites them for position quality data (GNSS satellite counts, dilution
        /// of precision, accuracy). They are kept verbatim so that a read→write
        /// cycle does not delete a field it did not model.
        ///
        /// The specification names one restriction this type does not enforce:
        /// members called `geometry` or `properties` are prohibited on a non-Feature
        /// GeoJSON object (RFC 7946 section 7.1). Enforcing it on *read* would make
        /// a file the specification's own validator accepts fail to open, which is
        /// the failure this type was written to end.
        pub other: Map<String, Value>,
    }

    impl Geolocation {
        /// A position with no altitude, no bounding box, and no foreign members.
        ///
        /// There is deliberately no [`Default`]: a default position is `0, 0`, a
        /// point in the Gulf of Guinea that is indistinguishable from a real
        /// measurement.
        ///
        /// The arguments are in GeoJSON's own order — longitude first.
        pub fn new(longitude: f64, latitude: f64) -> Geolocation {
            Geolocation {
                longitude,
                latitude,
                altitude: None,
                bbox: None,
                other: Map::new(),
            }
        }
    }

    /// The shape `core:geolocation` takes on the wire.
    ///
    /// Exists so [`Geolocation`] can name its components while still reading and
    /// writing the positional array GeoJSON requires.
    #[derive(Deserialize, Serialize)]
    struct GeolocationWire {
        #[serde(rename = "type")]
        geometry_type: PointType,

        coordinates: Vec<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        bbox: Option<Vec<f64>>,

        #[serde(flatten)]
        other: Map<String, Value>,
    }

    /// GeoJSON's `type` discriminator, which the schema pins to exactly `Point`.
    ///
    /// A one-variant enum rather than a checked `String`: the check becomes serde's
    /// job, and a `Geolocation` that is somehow not a Point is unrepresentable.
    #[derive(Deserialize, Serialize)]
    enum PointType {
        Point,
    }

    impl TryFrom<GeolocationWire> for Geolocation {
        // Unreachable as public API — `GeolocationWire` is private, so serde is the
        // only caller and it stringifies this immediately via `Error::custom`.
        type Error = String;

        fn try_from(wire: GeolocationWire) -> Result<Self, Self::Error> {
            let (longitude, latitude, altitude) = match wire.coordinates[..] {
                [longitude, latitude] => (longitude, latitude, None),
                [longitude, latitude, altitude] => (longitude, latitude, Some(altitude)),
                _ => {
                    return Err(format!(
                        "a GeoJSON Point's coordinates are longitude, latitude, and \
                         an optional altitude, so 2 or 3 numbers; got {}",
                        wire.coordinates.len()
                    ))
                }
            };

            Ok(Geolocation {
                longitude,
                latitude,
                altitude,
                bbox: wire.bbox,
                other: wire.other,
            })
        }
    }

    impl From<Geolocation> for GeolocationWire {
        fn from(geolocation: Geolocation) -> Self {
            let mut coordinates = vec![geolocation.longitude, geolocation.latitude];
            coordinates.extend(geolocation.altitude);

            GeolocationWire {
                geometry_type: PointType::Point,
                coordinates,
                bbox: geolocation.bbox,
                other: geolocation.other,
            }
        }
    }

    /// One entry of `core:extensions`: a SigMF extension namespace this Recording
    /// uses.
    ///
    /// The declaration is how a reader learns it needs to support a namespace
    /// *before* parsing. The schema requires all three fields and permits no
    /// others, which is what [`deny_unknown_fields`] mirrors.
    ///
    /// [`deny_unknown_fields`]: https://serde.rs/container-attrs.html#deny_unknown_fields
    #[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    pub struct Extension {
        /// The name of the extension namespace, e.g. `antenna` for `antenna:model`.
        pub name: String,

        /// The version of the extension namespace specification used.
        pub version: String,

        /// Whether a reader may ignore this extension.
        ///
        /// `false` means an application MUST support the extension in order to
        /// parse the Recording, and SHOULD report an error if it does not.
        ///
        /// Read that direction carefully, because the specification states it both
        /// ways. The schema's description of *this property* says the inverse — "If
        /// this field is `true`, the extension is REQUIRED to parse this Recording"
        /// — and it is simply wrong upstream: the field's name, the description of
        /// `core:extensions` itself, and the worked example accompanying it all
        /// agree with the reading above. Being a `bool` under either reading, this
        /// is a contradiction no validator can catch.
        pub optional: bool,
    }

    /// A typed view of one extension namespace's fields in the Global object.
    pub trait GlobalExtension {
        /// The namespace this extension's keys are prefixed with, without the
        /// colon — `antenna` for `antenna:model`.
        fn namespace() -> String;

        /// The version of the extension namespace specification this type models,
        /// as it should appear in `core:extensions`.
        fn version() -> String;

        /// Whether a reader may ignore this extension, for the declaration written
        /// into `core:extensions`. See [`Extension::optional`].
        ///
        /// Defaults to `true`, which suits a descriptive extension: a reader that
        /// skips `antenna:*` still gets every sample. Override it for an extension
        /// that a reader must understand to interpret the Dataset at all.
        fn optional() -> bool {
            true
        }
    }

    /// The `antenna` extension's Global fields: what was on the end of the coax.
    ///
    /// The one extension this crate ships a type for, and the reference example of
    /// [`GlobalExtension`]. Write it with [`GlobalMetadata::set_extension`], which
    /// also declares the namespace in `core:extensions`; read it back with
    /// [`GlobalMetadata::get_extension`].
    ///
    /// Only [`model`](Self::model) is required by the extension's schema, so
    /// [`Default`] plus struct update syntax is the intended way to build one.
    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct AntennaGlobal {
        /// Make and model — the extension's only required field.
        ///
        /// A catalogue entry rather than a category: `ARA CSB-16`, `Wellbrook
        /// ALA1530`. The category goes in [`antenna_type`](Self::antenna_type).
        #[serde(rename = "antenna:model")]
        pub model: String,

        /// The kind of antenna: `dipole`, `biconical`, `monopole`, and so on.
        ///
        /// Named `antenna_type` rather than `type`, which is a Rust keyword; the wire
        /// name is `antenna:type` regardless.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:type")]
        pub antenna_type: Option<String>,

        /// The low end of the antenna's operational range, in Hz.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:low_frequency")]
        pub low_frequency: Option<f64>,

        /// The high end of the antenna's operational range, in Hz.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:high_frequency")]
        pub high_frequency: Option<f64>,

        /// Gain in the direction of maximum radiation or reception, in dBi.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:gain")]
        pub gain: Option<f64>,

        /// Gain pattern in the horizontal plane, in dBi.
        ///
        /// The extension defines this as 0 to 359 degrees in 1-degree steps — so 360
        /// values, by position rather than by any index carried in the data. Nothing
        /// here enforces the length.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:horizontal_gain_pattern")]
        pub horizontal_gain_pattern: Option<Vec<f64>>,

        /// Gain pattern in the vertical plane, in dBi.
        ///
        /// Defined as -90 to +90 degrees in 1-degree steps — 181 values, positional,
        /// and again unenforced.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:vertical_gain_pattern")]
        pub vertical_gain_pattern: Option<Vec<f64>>,

        /// Horizontal 3 dB beamwidth, in degrees.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:horizontal_beam_width")]
        pub horizontal_beam_width: Option<f64>,

        /// Vertical 3 dB beamwidth, in degrees.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:vertical_beam_width")]
        pub vertical_beam_width: Option<f64>,

        /// Cross-polarization discrimination.
        ///
        /// The extension's schema gives this no unit, unlike every other numeric
        /// field here. Conventionally dB.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:cross_polar_discrimination")]
        pub cross_polar_discrimination: Option<f64>,

        /// Voltage standing wave ratio.
        ///
        /// The extension's schema says "in units of volts", which VSWR is not — it is
        /// a dimensionless ratio. Recorded here as upstream states it.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:voltage_standing_wave_ratio")]
        pub voltage_standing_wave_ratio: Option<f64>,

        /// Loss of the cable between antenna and preselector, in dB.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:cable_loss")]
        pub cable_loss: Option<f64>,

        /// Whether the antenna can be steered.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:steerable")]
        pub steerable: Option<bool>,

        /// Whether the antenna is mobile.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:mobile")]
        pub mobile: Option<bool>,

        /// Height of the antenna's phase centre above ground level, in metres.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:hagl")]
        pub hagl: Option<f64>,
    }

    impl Default for AntennaGlobal {
        fn default() -> AntennaGlobal {
            AntennaGlobal {
                model: "".to_string(),
                antenna_type: None,
                low_frequency: None,
                high_frequency: None,
                gain: None,
                horizontal_gain_pattern: None,
                vertical_gain_pattern: None,
                horizontal_beam_width: None,
                vertical_beam_width: None,
                cross_polar_discrimination: None,
                voltage_standing_wave_ratio: None,
                cable_loss: None,
                steerable: None,
                mobile: None,
                hagl: None,
            }
        }
    }

    impl GlobalExtension for AntennaGlobal {
        fn namespace() -> String {
            "antenna".to_string()
        }

        /// The version upstream's `extensions/antenna-schema.json` records in its
        /// own `$id`: `.../spec/1.0.0/extensions/antenna-schema`.
        fn version() -> String {
            "1.0.0".to_string()
        }
    }

    /// Anything that can go wrong opening, reading, or writing a Recording's files.
    ///
    /// The split from [`MetadataError`] follows the one the API already makes: a
    /// method that touches the filesystem returns this, and a method that only does
    /// arithmetic on a document returns [`MetadataError`]. Widening one type over
    /// both would put an `Io` variant in the return type of
    /// [`Metadata::capture_boundaries`], which reads no files and could never
    /// produce one — the same "field that can only ever hold a default" this crate
    /// spent a version removing, wearing an enum instead of a struct.
    ///
    /// [`MetadataError`] converts into this, so `?` carries either out of a method
    /// returning `Error`.
    #[derive(Debug, thiserror::Error)]
    #[non_exhaustive]
    pub enum Error {
        /// A file could not be opened, read, or written.
        ///
        /// The path is carried because [`std::io::Error`] has none of its own — "No
        /// such file or directory" is its whole message — and because the caller
        /// frequently does not have one either. Only [`SigMF::from_file`] touches a
        /// file the caller named; the Dataset is *derived* from the Metadata file's
        /// name, and [`SigMF::to_file`] derives both of its files from a basename.
        /// An error from any of those without a path names nothing the caller could
        /// look up.
        #[error("{path}: {source}", path = path.display())]
        Io {
            /// The file the operation was attempted on.
            path: PathBuf,
            /// What the operating system said.
            source: std::io::Error,
        },

        /// A Metadata document could not be parsed, or could not be serialized.
        ///
        /// This carries no path where [`Error::Io`] does, and the asymmetry is the
        /// point rather than an oversight: the only document this crate parses out
        /// of a file is the one [`SigMF::from_file`] was handed, so a caller holding
        /// a parse error is already holding the path it belongs to. The rest come
        /// from [`Metadata::from_json`] and [`Metadata::to_json`], which take and
        /// return a `String` and never learn of a file at all. `serde_json`'s own
        /// message carries the line and column, which is the part that is genuinely
        /// hard to recover at the call site.
        #[error(transparent)]
        Json(#[from] serde_json::Error),

        /// The Metadata is not a description of the Dataset that was asked for.
        #[error(transparent)]
        Metadata(#[from] MetadataError),
    }

    /// A Metadata document says something that does not work.
    ///
    /// Every variant here is a statement about a document's *contents* — either
    /// self-contradictory, or contradicting a Dataset, or contradicting what the
    /// caller asked for. Nothing here reads a file; see [`Error`] for that.
    #[derive(Debug, thiserror::Error)]
    #[non_exhaustive]
    pub enum MetadataError {
        /// A Recording declaring `core:num_channels` other than 1 met the typed
        /// sample API, which has no way to express it in either direction.
        #[error(
            "cannot use a typed sample buffer for a Dataset with `core:num_channels` = {0}: \
             such a buffer is one channel, and interleaving several into it would leave \
             `core:datatype` describing something other than the bytes. The specification \
             recommends SigMF Collections over `core:num_channels` for multi-channel IQ, \
             for widest application support"
        )]
        MultiChannelDataset(u64),

        /// A Dataset was asked for as a Rust type that its `core:datatype` does not
        /// describe.
        #[error(
            "cannot read a `{stored}` Dataset as `{requested}`: `core:datatype` is the \
             Recording's own account of what its bytes mean, and reading them as anything \
             else yields plausible noise rather than an error"
        )]
        DatatypeMismatch {
            /// What the Recording says its samples are.
            stored: DataFormat,
            /// What the caller asked to read them as.
            requested: DataFormat,
        },

        /// The samples of a Recording that has no Dataset file were asked for.
        #[error(
            "this Recording has no Dataset file: it is either `core:metadata_only`, or its \
             Metadata file is not named `<basename>.sigmf-meta` and so has no Dataset that \
             can be named from it, or it has not been written yet"
        )]
        NoDataset,

        /// A Captures segment describes bytes the Dataset does not have.
        #[error(
            "capture {index} covers bytes {start}..{end} of a Dataset that is {dataset_len} \
             bytes long. The specification requires `captures` to be sorted by \
             `core:sample_start` ascending; a Recording whose segments are not sorted lands \
             here too"
        )]
        CaptureOutOfBounds {
            /// Position of the offending segment in the `captures` array.
            index: usize,
            /// First byte the segment claims.
            start: u64,
            /// One past the last byte the segment claims.
            end: u64,
            /// Size of the Dataset the segment claims them from.
            dataset_len: u64,
        },

        /// `core:dataset` names something other than a file beside the Metadata
        /// file.
        #[error(
            "`core:dataset` is {0:?}, which is not a plain filename. The specification says \
             this field \"only includes the filename, not directory\", and the Dataset \
             \"must be in the same directory as the .sigmf-meta file\""
        )]
        DatasetPathEscapesDirectory(String),

        /// A Captures segment's bytes are not a whole number of samples.
        #[error(
            "a capture holds {bytes} bytes, which is not a whole number of `{datatype}` \
             samples of {} bytes each", datatype.size()
        )]
        PartialSample {
            /// Length of the segment.
            bytes: u64,
            /// The format whose sample width does not divide it.
            datatype: DataFormat,
        },

        /// `core:trailing_bytes` claims more of the Dataset than the Dataset has.
        #[error(
            "`core:trailing_bytes` is {trailing}, but the Dataset is only {dataset_len} \
             bytes long, so there are no sample bytes in front of them"
        )]
        TrailingBytesExceedDataset {
            /// What `core:trailing_bytes` says is not sample data.
            trailing: u64,
            /// Size of the whole Dataset.
            dataset_len: u64,
        },

        /// A `core:sample_start` is too far into the Dataset to be a byte offset.
        ///
        /// A sample index near [`u64::MAX`] names a byte past the end of the
        /// addressable file. No Dataset is that large; a document saying otherwise
        /// is corrupt or hostile, and the alternative to this error is a release
        /// build wrapping the multiplication and answering with a small, plausible,
        /// wrong offset.
        #[error(
            "capture {index}: `core:sample_start` {sample_start} is further into the Dataset \
             than a byte offset can reach"
        )]
        SampleStartOutOfRange {
            /// Position of the offending segment in the `captures` array.
            index: usize,
            /// The sample index that cannot be converted to a byte offset.
            sample_start: u64,
        },

        /// An extension type could not be serialized into the Global object.
        #[error("extension data for the `{namespace}` namespace could not be serialized")]
        ExtensionNotSerializable {
            /// The namespace the type declared.
            namespace: String,
            /// Why `serde_json` refused it.
            source: serde_json::Error,
        },

        /// An extension type serialized to something other than a JSON object.
        ///
        /// Extension data is a set of `namespace:key` fields merged into the Global
        /// object, so a type that serializes to an array, a string, or a number has
        /// no fields to merge and no key to merge them under.
        #[error(
            "extension data for the `{namespace}` namespace serialized to a JSON {found}, \
             but the Global object can only be extended with named fields, so an extension \
             must serialize to an object"
        )]
        ExtensionNotAnObject {
            /// The namespace the type declared.
            namespace: String,
            /// What it serialized to instead, named as JSON names it.
            found: &'static str,
        },
    }

    impl GlobalMetadata {
        /// A global object carrying the two fields the specification requires —
        /// `core:datatype` and `core:version` — and nothing else.
        ///
        /// There is deliberately no [`Default`]: the schema requires both of these,
        /// and neither has a defensible default. A datatype cannot be guessed, and
        /// defaulting the version to whatever the crate happens to implement is
        /// exactly right for a recording being *written* — which is why it is set
        /// here — but would be a fabrication anywhere else.
        ///
        /// # Examples
        ///
        /// ```
        /// use sigmf::GlobalMetadata;
        ///
        /// let global = GlobalMetadata::new("cf32_le".parse()?);
        /// assert_eq!(global.datatype.to_string(), "cf32_le");
        /// assert_eq!(global.version, sigmf::SIGMF_VERSION);
        /// # Ok::<(), sigmf::ParseDataFormatError>(())
        /// ```
        pub fn new(datatype: DataFormat) -> GlobalMetadata {
            GlobalMetadata {
                version: SIGMF_VERSION.to_string(),
                datatype,
                sample_rate: None,
                num_channels: None,
                sha512: None,
                offset: None,
                description: None,
                author: None,
                meta_doi: None,
                data_doi: None,
                recorder: None,
                license: None,
                hw: None,
                geolocation: None,
                extensions: None,
                collection: None,
                metadata_only: None,
                dataset: None,
                trailing_bytes: None,
                other: Map::new(),
            }
        }

        /// Read one extension namespace's fields, or `None` if this Recording
        /// carries none of them.
        ///
        /// Presence is decided by the data — whether any `namespace:` key is
        /// present — and not by whether `core:extensions` declares the namespace.
        /// Undeclared extension data is a spec violation on the part of whoever
        /// wrote the file (one this crate itself committed until it learned to
        /// declare what it writes), but the data is there and refusing to read it
        /// would help nobody.
        pub fn get_extension<T: GlobalExtension + serde::de::DeserializeOwned>(
            &self,
        ) -> Result<Option<T>, serde_json::Error> {
            let namespace_pattern = T::namespace() + ":";
            if !self
                .other
                .keys()
                .any(|k| k.starts_with(namespace_pattern.as_str()))
            {
                return Ok(None);
            }
            serde_json::from_value(serde_json::json!(self.other)).map(Some)
        }

        /// Write one extension namespace's fields, replacing any already present,
        /// and declare the namespace in `core:extensions`.
        ///
        /// The declaration is not a nicety: the specification is explicit that
        /// `core:extensions` is how a reader learns it needs to support a namespace
        /// before parsing, so writing `antenna:model` without it emits a file a
        /// conformant reader cannot know how to handle.
        ///
        /// # Examples
        ///
        /// ```
        /// use sigmf::{AntennaGlobal, GlobalMetadata};
        ///
        /// let mut global = GlobalMetadata::new("cf32_le".parse()?);
        /// global.set_extension(AntennaGlobal {
        ///     model: "Wellbrook ALA1530".to_string(),
        ///     ..Default::default()
        /// })?;
        ///
        /// let declared = global.extensions.as_ref().expect("the namespace is declared");
        /// assert_eq!(declared[0].name, "antenna");
        /// # Ok::<(), Box<dyn std::error::Error>>(())
        /// ```
        pub fn set_extension<T: GlobalExtension + serde::Serialize>(
            &mut self,
            val: T,
        ) -> Result<(), MetadataError> {
            let serialized = serde_json::to_value(val).map_err(|source| {
                MetadataError::ExtensionNotSerializable {
                    namespace: T::namespace(),
                    source,
                }
            })?;

            let Value::Object(fields) = serialized else {
                return Err(MetadataError::ExtensionNotAnObject {
                    namespace: T::namespace(),
                    found: json_type_name(&serialized),
                });
            };

            let namespace_pattern = T::namespace() + ":";
            self.other
                .retain(|k, _| !k.starts_with(namespace_pattern.as_str()));
            self.other.extend(fields);
            self.declare_extension::<T>();
            Ok(())
        }

        /// Record `T`'s namespace in `core:extensions`, replacing any existing
        /// declaration of the same namespace rather than duplicating it.
        fn declare_extension<T: GlobalExtension>(&mut self) {
            let declaration = Extension {
                name: T::namespace(),
                version: T::version(),
                optional: T::optional(),
            };

            let declared = self.extensions.get_or_insert_with(Vec::new);
            match declared.iter_mut().find(|e| e.name == declaration.name) {
                Some(existing) => *existing = declaration,
                None => declared.push(declaration),
            }
        }

        /// Remove one extension namespace's fields and its `core:extensions`
        /// declaration.
        ///
        /// The declaration goes with the data: leaving it behind would announce an
        /// extension the Recording no longer uses, which for a non-`optional` one
        /// tells a reader to refuse a file it could in fact parse.
        ///
        /// Removing a namespace that was never present is not an error, and neither
        /// is anything else: this returns nothing because it cannot fail. It used to
        /// return `Result<(), MetadataError>` with a single `Ok(())` exit and no
        /// fallible call in the body — a `?` at every call site, standing guard over
        /// an error that had no way to exist.
        pub fn delete_extension<T: GlobalExtension>(&mut self) {
            let namespace_pattern = T::namespace() + ":";
            self.other
                .retain(|k, _| !k.starts_with(namespace_pattern.as_str()));

            if let Some(declared) = &mut self.extensions {
                declared.retain(|e| e.name != T::namespace());
            }
        }
    }

    /// The byte order of a multi-byte sample.
    ///
    /// Only the multi-byte [`DataType`] variants carry one. A single byte has no
    /// byte order to state, and the specification's datatype grammar reflects that
    /// by omitting the suffix for `i8` and `u8`.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum Endianness {
        /// Most significant byte first, spelled `_be`.
        BigEndian,
        /// Least significant byte first, spelled `_le`.
        LittleEndian,
    }

    impl Endianness {
        /// The suffix this byte order is spelled with in a datatype string.
        fn suffix(self) -> &'static str {
            match self {
                Endianness::BigEndian => "_be",
                Endianness::LittleEndian => "_le",
            }
        }
    }

    /// The type of one component of a sample.
    ///
    /// These are exactly the eight the specification permits — note the absence of
    /// 64-bit integers, which is deliberate and matches the schema.
    ///
    /// Byte order is part of the variant rather than a sibling field because it is
    /// only meaningful for the multi-byte types. This is the correlation the
    /// schema's `core:datatype` regex cannot express, and it is why [`DataFormat`]'s
    /// parser rejects both `cf32` and `ri8_le`.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum DataType {
        /// 32-bit IEEE-754 float, spelled `f32_le` or `f32_be`.
        F32(Endianness),
        /// 64-bit IEEE-754 float, spelled `f64_le` or `f64_be`.
        F64(Endianness),
        /// Signed 32-bit integer, spelled `i32_le` or `i32_be`.
        I32(Endianness),
        /// Signed 16-bit integer, spelled `i16_le` or `i16_be`.
        I16(Endianness),
        /// Unsigned 32-bit integer, spelled `u32_le` or `u32_be`.
        U32(Endianness),
        /// Unsigned 16-bit integer, spelled `u16_le` or `u16_be`.
        U16(Endianness),
        /// Signed 8-bit integer, spelled `i8`.
        I8,
        /// Unsigned 8-bit integer, spelled `u8`.
        U8,
    }

    impl DataType {
        /// The width of one component in bytes.
        pub fn size(&self) -> u64 {
            match self {
                DataType::F32(_) => 4,
                DataType::F64(_) => 8,
                DataType::I32(_) => 4,
                DataType::I16(_) => 2,
                DataType::U32(_) => 4,
                DataType::U16(_) => 2,
                DataType::I8 => 1,
                DataType::U8 => 1,
            }
        }
    }

    impl fmt::Display for DataType {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                DataType::F32(e) => write!(f, "f32{}", e.suffix()),
                DataType::F64(e) => write!(f, "f64{}", e.suffix()),
                DataType::I32(e) => write!(f, "i32{}", e.suffix()),
                DataType::I16(e) => write!(f, "i16{}", e.suffix()),
                DataType::U32(e) => write!(f, "u32{}", e.suffix()),
                DataType::U16(e) => write!(f, "u16{}", e.suffix()),
                DataType::I8 => f.write_str("i8"),
                DataType::U8 => f.write_str("u8"),
            }
        }
    }

    /// Whether each sample is one component or an interleaved in-phase/quadrature
    /// pair.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum NumberType {
        /// One component per sample, spelled `r`.
        Real,
        /// Two interleaved components per sample, spelled `c`.
        Complex,
    }

    impl fmt::Display for NumberType {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                NumberType::Real => "r",
                NumberType::Complex => "c",
            })
        }
    }

    /// The format of the samples in a Dataset file: the value of `core:datatype`.
    ///
    /// This is a claim about the bytes, and the most consequential field in a
    /// recording — reading `cf32_le` bytes as `ci16_le` yields not an error but
    /// plausible-looking noise. It is parsed, not stored as a string, so a value
    /// that cannot describe any real byte layout cannot exist in a [`Metadata`].
    ///
    /// # Examples
    ///
    /// ```
    /// use sigmf::{DataFormat, DataType, Endianness, NumberType};
    ///
    /// let format: DataFormat = "cf32_le".parse()?;
    /// assert_eq!(format.number_type, NumberType::Complex);
    /// assert_eq!(format.data_type, DataType::F32(Endianness::LittleEndian));
    ///
    /// // Complex doubles the width: two f32 components per sample.
    /// assert_eq!(format.size(), 8);
    ///
    /// // Display is the exact inverse of the parse.
    /// assert_eq!(format.to_string(), "cf32_le");
    /// # Ok::<(), sigmf::ParseDataFormatError>(())
    /// ```
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub struct DataFormat {
        /// Whether samples are real or interleaved complex pairs.
        pub number_type: NumberType,
        /// The type of each component of a sample.
        pub data_type: DataType,
    }

    impl DataFormat {
        /// The width of one whole sample in bytes, counting both components of a
        /// complex pair.
        pub fn size(&self) -> u64 {
            self.data_type.size()
                * match self.number_type {
                    NumberType::Real => 1,
                    NumberType::Complex => 2,
                }
        }

        /// The `core:datatype` a Dataset of `S` samples written in `endianness`
        /// carries.
        ///
        /// This is the derivation [`SigMF::to_file`] performs, exposed because a
        /// caller may reasonably want to know what it is about to write. Note what
        /// is missing from the signature: there is no sample buffer, because the
        /// answer is a function of the *type*, and no fallible path, because there
        /// is no input here that could be wrong.
        ///
        /// # Examples
        ///
        /// ```
        /// use sigmf::num_complex::Complex;
        /// use sigmf::{DataFormat, Endianness::{BigEndian, LittleEndian}};
        ///
        /// assert_eq!(DataFormat::of::<Complex<f32>>(LittleEndian).to_string(), "cf32_le");
        /// assert_eq!(DataFormat::of::<i16>(BigEndian).to_string(), "ri16_be");
        ///
        /// // One byte has no byte order, and the datatype does not pretend it does.
        /// assert_eq!(DataFormat::of::<u8>(BigEndian).to_string(), "ru8");
        /// ```
        pub fn of<S: Sample>(endianness: Endianness) -> DataFormat {
            S::data_format(endianness)
        }

        /// The byte order the samples are stored in, or `None` for the one-byte
        /// component types, which have none.
        ///
        /// `None` is not "unknown" — it is the correlation `core:datatype`'s grammar
        /// encodes and its schema regex cannot express. There is no `ri8_le` to
        /// parse, so an `ri8` Recording has no byte order to report, and one byte
        /// reads the same either way.
        ///
        /// # Examples
        ///
        /// ```
        /// use sigmf::{DataFormat, Endianness};
        ///
        /// assert_eq!("cf32_be".parse::<DataFormat>()?.endianness(), Some(Endianness::BigEndian));
        /// assert_eq!("ri8".parse::<DataFormat>()?.endianness(), None);
        /// # Ok::<(), sigmf::ParseDataFormatError>(())
        /// ```
        pub fn endianness(&self) -> Option<Endianness> {
            match self.data_type {
                DataType::F32(e)
                | DataType::F64(e)
                | DataType::I32(e)
                | DataType::I16(e)
                | DataType::U32(e)
                | DataType::U16(e) => Some(e),
                DataType::I8 | DataType::U8 => None,
            }
        }
    }

    impl fmt::Display for DataFormat {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}{}", self.number_type, self.data_type)
        }
    }

    /// A Rust type that can be the sample type of a SigMF Dataset.
    ///
    /// Implemented for the eight component types the specification permits — `f32`,
    /// `f64`, `i32`, `i16`, `u32`, `u16`, `i8`, `u8` — each on its own, which is a
    /// real (`r`) datatype, and each wrapped in [`Complex`], which is a complex
    /// (`c`) one. Those sixteen are the whole of SigMF's sample vocabulary.
    ///
    /// # Why this is sealed
    ///
    /// The trait cannot be implemented outside this crate, and that is not
    /// housekeeping. `core:datatype` is a claim about the bytes of the Dataset, and
    /// it is the claim every reader trusts in order to interpret them — read
    /// `cf32_le` bytes as `ci16_le` and there is no error, only plausible noise.
    /// Sealing is what earns [`SigMF::to_file`] the right to *derive* that claim
    /// from `S` instead of trusting a caller to state it: a downstream
    /// `impl Sample for MyType` announcing `f32` while occupying eight bytes would
    /// turn the derivation into a lie told by a signature that looks like it
    /// checked. The specification's list of eight is closed, so there is nothing
    /// legitimate to add here anyway.
    pub trait Sample: Copy + private::Sealed {}

    mod private {
        use super::{DataFormat, Endianness};

        /// What [`Sample`](super::Sample) actually provides.
        ///
        /// Unnameable downstream, which makes `Sample` both unimplementable and
        /// free to change: everything here is an implementation detail of the write
        /// path, and the public surface a caller needs is
        /// [`DataFormat::of`](super::DataFormat::of).
        pub trait Sealed {
            /// The `core:datatype` a Dataset of these samples carries.
            fn data_format(endianness: Endianness) -> DataFormat;

            /// Append this sample's bytes, in `endianness`, to `out`.
            ///
            /// Infallible, and writing to a buffer rather than a sink, because the
            /// whole Dataset is assembled in memory before any of it is written —
            /// the samples are already in memory when they arrive, and the checksum
            /// needs a second look at them.
            fn encode(self, endianness: Endianness, out: &mut Vec<u8>);

            /// Read one sample from exactly [`DataFormat::size`](super::DataFormat::size)
            /// bytes of Dataset, the inverse of [`encode`](Self::encode).
            ///
            /// Infallible because both facts that could make it fail have already
            /// been established by the only caller: that `Self` is what
            /// `core:datatype` says, and that `bytes` is one whole sample.
            ///
            /// # Panics
            ///
            /// If `bytes` is not exactly one sample wide.
            fn decode(endianness: Endianness, bytes: &[u8]) -> Self;
        }
    }

    /// Implements [`Sample`] for a component type and for its complex pair.
    ///
    /// `$data_type` maps a byte order to the [`DataType`] the component is spelled
    /// with. For the multi-byte types that is the variant's own constructor. The
    /// two 8-bit types pass a closure that discards the byte order, because one
    /// byte has none: the specification's datatype grammar has no `i8_le`, and
    /// [`DataFormat`]'s parser rejects it.
    macro_rules! impl_sample {
        ($component:ty, $data_type:expr) => {
            impl private::Sealed for $component {
                fn data_format(endianness: Endianness) -> DataFormat {
                    DataFormat {
                        number_type: NumberType::Real,
                        data_type: $data_type(endianness),
                    }
                }

                fn encode(self, endianness: Endianness, out: &mut Vec<u8>) {
                    match endianness {
                        Endianness::LittleEndian => out.extend_from_slice(&self.to_le_bytes()),
                        Endianness::BigEndian => out.extend_from_slice(&self.to_be_bytes()),
                    }
                }

                fn decode(endianness: Endianness, bytes: &[u8]) -> Self {
                    let mut component = [0u8; std::mem::size_of::<$component>()];
                    component.copy_from_slice(bytes);
                    match endianness {
                        Endianness::LittleEndian => Self::from_le_bytes(component),
                        Endianness::BigEndian => Self::from_be_bytes(component),
                    }
                }
            }

            impl Sample for $component {}

            impl private::Sealed for Complex<$component> {
                fn data_format(endianness: Endianness) -> DataFormat {
                    DataFormat {
                        number_type: NumberType::Complex,
                        data_type: $data_type(endianness),
                    }
                }

                fn encode(self, endianness: Endianness, out: &mut Vec<u8>) {
                    // In-phase then quadrature: the interleaving `c` denotes.
                    self.re.encode(endianness, out);
                    self.im.encode(endianness, out);
                }

                fn decode(endianness: Endianness, bytes: &[u8]) -> Self {
                    let (re, im) = bytes.split_at(std::mem::size_of::<$component>());
                    Complex::new(
                        <$component as private::Sealed>::decode(endianness, re),
                        <$component as private::Sealed>::decode(endianness, im),
                    )
                }
            }

            impl Sample for Complex<$component> {}
        };
    }

    impl_sample!(f32, DataType::F32);
    impl_sample!(f64, DataType::F64);
    impl_sample!(i32, DataType::I32);
    impl_sample!(i16, DataType::I16);
    impl_sample!(u32, DataType::U32);
    impl_sample!(u16, DataType::U16);
    impl_sample!(i8, |_| DataType::I8);
    impl_sample!(u8, |_| DataType::U8);

    /// The reason a string is not a valid `core:datatype`.
    ///
    /// Deliberately opaque, mirroring [`std::num::ParseIntError`]: the useful
    /// content is the [`Display`](fmt::Display) message, which names both the
    /// offending input and what was expected in its place.
    #[derive(Debug, PartialEq, Eq, Clone)]
    pub struct ParseDataFormatError {
        input: Box<str>,
        kind: ParseDataFormatErrorKind,
    }

    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    enum ParseDataFormatErrorKind {
        NumberType,
        SampleType,
        MissingEndianness,
        Endianness,
        RedundantEndianness,
    }

    impl fmt::Display for ParseDataFormatError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "invalid SigMF datatype `{}`: ", self.input)?;
            match self.kind {
                ParseDataFormatErrorKind::NumberType => {
                    f.write_str("must begin with `r` (real) or `c` (complex)")
                }
                ParseDataFormatErrorKind::SampleType => f.write_str(
                    "unknown sample type; the specification permits \
                     f32, f64, i32, i16, u32, u16, i8, u8",
                ),
                ParseDataFormatErrorKind::MissingEndianness => f.write_str(
                    "a multi-byte sample type must state its byte order \
                     with an `_le` or `_be` suffix",
                ),
                ParseDataFormatErrorKind::Endianness => {
                    f.write_str("expected a byte order suffix of `_le` or `_be`")
                }
                ParseDataFormatErrorKind::RedundantEndianness => f.write_str(
                    "a single-byte sample type has no byte order and must not \
                     carry an `_le` or `_be` suffix",
                ),
            }
        }
    }

    // Hand-written, and staying that way: the default `source()` of `None` is the
    // truth here, because a datatype that does not parse has no underlying cause —
    // the string is simply not one of twenty-eight spellings.
    impl std::error::Error for ParseDataFormatError {}

    impl std::str::FromStr for DataFormat {
        type Err = ParseDataFormatError;

        /// Parse a `core:datatype` string such as `cf32_le`.
        ///
        /// Total over its input and stricter than the schema's regex in three ways,
        /// each of which the regex permits only because it cannot say otherwise:
        ///
        /// - **Trailing garbage is rejected.** The schema's pattern carries no `$`
        ///   anchor, so `cf32_le_GARBAGE` satisfies it by matching the `cf32_le`
        ///   prefix and ignoring the rest.
        /// - **A bare `cf32` is rejected.** The suffix is optional in the pattern
        ///   for the sake of `i8`/`u8`; a regex cannot make it conditional on the
        ///   width. Guessing the byte order of an `f32` would silently produce
        ///   byte-swapped garbage, which is the failure this type exists to prevent.
        /// - **`ri8_le` is rejected.** Accepting it would mean [`Display`](fmt::Display)
        ///   emitting `ri8` for a value parsed from `ri8_le`, so reading and
        ///   rewriting a file would quietly alter a required field.
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let fail = |kind| ParseDataFormatError {
                input: s.into(),
                kind,
            };

            let (number_type, rest) = match s.as_bytes().first() {
                Some(b'r') => (NumberType::Real, &s[1..]),
                Some(b'c') => (NumberType::Complex, &s[1..]),
                _ => return Err(fail(ParseDataFormatErrorKind::NumberType)),
            };

            // Splitting on the first `_` is what makes the parse total: everything
            // after it must be exactly an endianess suffix, so trailing garbage has
            // nowhere to hide.
            let (base, suffix) = match rest.split_once('_') {
                Some((base, suffix)) => (base, Some(suffix)),
                None => (rest, None),
            };

            let data_type = match base {
                "i8" | "u8" => {
                    if suffix.is_some() {
                        return Err(fail(ParseDataFormatErrorKind::RedundantEndianness));
                    }
                    if base == "i8" {
                        DataType::I8
                    } else {
                        DataType::U8
                    }
                }
                "f32" | "f64" | "i32" | "i16" | "u32" | "u16" => {
                    let endianess = match suffix {
                        Some("le") => Endianness::LittleEndian,
                        Some("be") => Endianness::BigEndian,
                        Some(_) => return Err(fail(ParseDataFormatErrorKind::Endianness)),
                        None => return Err(fail(ParseDataFormatErrorKind::MissingEndianness)),
                    };
                    match base {
                        "f32" => DataType::F32(endianess),
                        "f64" => DataType::F64(endianess),
                        "i32" => DataType::I32(endianess),
                        "i16" => DataType::I16(endianess),
                        "u32" => DataType::U32(endianess),
                        _ => DataType::U16(endianess),
                    }
                }
                _ => return Err(fail(ParseDataFormatErrorKind::SampleType)),
            };

            Ok(DataFormat {
                number_type,
                data_type,
            })
        }
    }

    impl Serialize for DataFormat {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    impl<'de> Deserialize<'de> for DataFormat {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct DataFormatVisitor;

            impl serde::de::Visitor<'_> for DataFormatVisitor {
                type Value = DataFormat;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a SigMF datatype string such as `cf32_le`")
                }

                fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                    v.parse().map_err(E::custom)
                }
            }

            deserializer.deserialize_str(DataFormatVisitor)
        }
    }
}

pub use sigmf::*;

#[cfg(test)]
mod test;
