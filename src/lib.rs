/// The complex-number type this crate's sample vocabulary is built on.
///
/// Re-exported because it is a *public* dependency: `Sample` is implemented for
/// `num_complex::Complex<f32>` and not for some structurally identical type from
/// another copy of the crate, so a caller whose `num-complex` resolves to a
/// different major than ours would find `to_file` mysteriously unwilling to take
/// their samples. Reaching for `sigmf::num_complex` instead of a direct dependency
/// makes that impossible to get wrong.
pub use num_complex;

const SIGMF_ARCHIVE_EXT: &'static str = ".sigmf";
const SIGMF_METADATA_EXT: &'static str = ".sigmf-meta";
const SIGMF_DATASET_EXT: &'static str = ".sigmf-data";
const SIGMF_COLLECTION_EXT: &'static str = ".sigmf-collection";

pub mod sigmf {
    use crate::{SIGMF_DATASET_EXT, SIGMF_METADATA_EXT};
    use core::fmt::Debug;
    use num_complex::Complex;
    use serde_json::de::Read;
    use serde_json::Value;
    use sha2::{Digest, Sha512};
    use std::collections::BTreeMap as Map;
    use std::ffi::OsStr;
    use std::fmt::{self, Write as _};
    use std::ops::Range;
    use std::{
        error::Error,
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

    #[derive(Debug)]
    pub struct SigMF {
        pub metadata: Metadata,
        // captures: Vec<CaptureMetadata>,
        datafile: Option<PathBuf>,
    }

    impl SigMF {
        /// Open a Recording, given the path of its `.sigmf-meta` file.
        ///
        /// The Dataset is not read here, or even opened — only its path is worked
        /// out, by the rules in [`dataset_path`]. Reading the metadata of a
        /// hundred-gigabyte Recording therefore costs the size of its sidecar, and
        /// [`samples`](Self::samples) is the call that goes to disk for the rest.
        ///
        /// # Errors
        ///
        /// Returns an I/O error if the Metadata file cannot be read, a
        /// deserialization error if it is not a valid SigMF document — which
        /// includes a `core:datatype` that describes no possible bytes — or
        /// [`MetadataError::DatasetPathEscapesDirectory`] if `core:dataset` names
        /// something other than a file beside the Metadata file.
        pub fn from_file<T: AsRef<Path>>(path: T) -> Result<Self, Box<dyn Error>> {
            let path = path.as_ref();
            let metadata_file = fs::File::open(path)?;
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
        /// measure, an I/O error if it cannot be measured, or
        /// [`MetadataError::CaptureOutOfBounds`] if the Metadata describes bytes
        /// the Dataset does not have.
        pub fn capture_boundaries(&self) -> Result<Vec<Range<u64>>, Box<dyn Error>> {
            let path = self.datafile.as_ref().ok_or(MetadataError::NoDataset)?;
            let dataset_len = fs::metadata(path)?.len();
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
        /// number of samples, or an I/O error.
        pub fn samples<S: Sample>(&self) -> Result<Vec<S>, Box<dyn Error>> {
            let datatype = self.metadata.global.datatype;

            // A one-byte component has no byte order, so for `ri8`/`ru8` the
            // argument here cannot change the answer; for every other type the
            // stored order is the only one that could possibly match.
            let endianness = datatype.endianness().unwrap_or(Endianess::LittleEndian);
            let requested = DataFormat::of::<S>(endianness);
            if requested != datatype {
                return Err(Box::new(MetadataError::DatatypeMismatch {
                    stored: datatype,
                    requested,
                }));
            }

            // The mirror of `to_file_with`'s refusal, and for the same reason: with
            // several channels interleaved into the Dataset, one element of a
            // `Vec<S>` is one channel's sample, and the Vec says nothing about
            // which. Deinterleaving wants a return type that admits channels exist.
            if let Some(channels) = self.metadata.global.num_channels {
                if channels != 1 {
                    return Err(Box::new(MetadataError::MultiChannelDataset(channels)));
                }
            }

            let path = self.datafile.as_ref().ok_or(MetadataError::NoDataset)?;
            let data = fs::read(path)?;
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
                    return Err(Box::new(MetadataError::PartialSample {
                        bytes: bytes.len() as u64,
                        datatype,
                    }));
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
        pub fn to_file<S: Sample, P: AsRef<Path>>(
            &mut self,
            basename: P,
            samples: &[S],
        ) -> Result<(), Box<dyn Error>> {
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
        /// Collections instead), or an I/O error if either file cannot be written.
        pub fn to_file_with<S: Sample, P: AsRef<Path>>(
            &mut self,
            basename: P,
            samples: &[S],
            options: WriteOptions,
        ) -> Result<(), Box<dyn Error>> {
            // A `&[S]` is one channel by construction: nothing in the slice can say
            // where one channel ends and the next begins, so honouring
            // `core:num_channels > 1` would mean writing a datatype that describes
            // something other than the bytes.
            if let Some(channels) = self.metadata.global.num_channels {
                if channels != 1 {
                    return Err(Box::new(MetadataError::MultiChannelDataset(channels)));
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

            fs::write(&data_path, &data)?;
            fs::write(&metadata_path, self.metadata.to_str()?)?;

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

    /// The knobs [`SigMF::to_file_with`] turns, with sane values from [`Default`].
    ///
    /// Both defaults are safe to take blind, and it is worth saying why, because
    /// defaulting a field of `core:datatype` would be alarming in any other design:
    /// whatever byte order this picks is the byte order the emitted datatype
    /// *states*. The choice cannot make a Recording lie about itself; at worst it
    /// makes one inconvenient to a reader that wanted the other order.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct WriteOptions {
        endianness: Endianess,
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
                endianness: Endianess::LittleEndian,
                checksum: true,
            }
        }
    }

    impl WriteOptions {
        /// Write samples in this byte order, and say so in `core:datatype`.
        pub fn endianness(mut self, endianness: Endianess) -> Self {
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

    #[derive(Debug, Deserialize, Serialize)]
    pub struct Metadata {
        pub global: GlobalMetadata,
        pub captures: Vec<CaptureMetadata>,
        pub annotations: Vec<AnnotationMetadata>,
    }

    impl Metadata {
        /// Parse a `.sigmf-meta` document.
        ///
        /// Nothing here reads the Dataset. This function used to take its bytes so
        /// that it could compute capture boundaries on the way past; that is now
        /// [`capture_boundaries`](Self::capture_boundaries)' job, which asks only
        /// for a length.
        pub fn from_json(s: &str) -> Result<Self, Box<dyn Error>> {
            Ok(serde_json::from_str(s)?)
        }

        pub fn to_str(&self) -> Result<String, Box<dyn Error>> {
            let res = serde_json::to_string_pretty(self)?;
            Ok(res)
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
        pub fn capture_boundaries(
            &self,
            dataset_len: u64,
        ) -> Result<Vec<Range<u64>>, MetadataError> {
            let trailing = self.global.trailing_bytes.unwrap_or(0);
            let last_sample_byte = dataset_len.checked_sub(trailing).ok_or_else(|| {
                MetadataError::Internal(format!(
                    "`core:trailing_bytes` is {trailing}, but the Dataset is only \
                     {dataset_len} bytes long",
                ))
            })?;

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
                let byte_of = |sample: u64| -> Result<u64, MetadataError> {
                    sample_size
                        .checked_mul(sample)
                        .and_then(|offset| offset.checked_add(headers))
                        .ok_or_else(|| {
                            MetadataError::Internal(format!(
                                "capture {index}: `core:sample_start` {sample} is further into \
                                 the Dataset than a byte offset can reach",
                            ))
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

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct GlobalMetadata {
        #[serde(rename = "core:datatype")]
        pub datatype: DataFormat,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:sample_rate")]
        pub sample_rate: Option<f64>,

        #[serde(rename = "core:version")]
        pub version: String,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:num_channels")]
        pub num_channels: Option<u64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:sha512")]
        pub sha512: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:offset")]
        pub offset: Option<u64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:description")]
        pub description: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:author")]
        pub author: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:meta_doi")]
        pub meta_doi: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:data_doi")]
        pub data_doi: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:recorder")]
        pub recorder: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:license")]
        pub license: Option<String>,

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

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:metadata_only")]
        pub metadata_only: Option<bool>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:dataset")]
        pub dataset: Option<String>,

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

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct CaptureMetadata {
        #[serde(rename = "core:sample_start")]
        pub sample_start: u64,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:global_index")]
        pub global_index: Option<u64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:frequency")]
        pub frequency: Option<f64>,

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

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct AnnotationMetadata {
        #[serde(rename = "core:sample_start")]
        pub sample_start: u64,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:sample_count")]
        pub sample_count: Option<u64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:freq_lower_edge")]
        pub freq_lower_edge: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:freq_upper_edge")]
        pub freq_upper_edge: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:label")]
        pub label: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:generator")]
        pub generator: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:comment")]
        pub comment: Option<String>,

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
    /// use sigmf::sigmf::Geolocation;
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

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct AntennaGlobal {
        #[serde(rename = "antenna:model")]
        pub model: String,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:type")]
        pub antenna_type: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:low_frequency")]
        pub low_frequency: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:high_frequency")]
        pub high_frequency: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:gain")]
        pub gain: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:horizontal_gain_pattern")]
        pub horizontal_gain_pattern: Option<Vec<f64>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:vertical_gain_pattern")]
        pub vertical_gain_pattern: Option<Vec<f64>>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:horizontal_beam_width")]
        pub horizontal_beam_width: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:vertical_beam_width")]
        pub vertical_beam_width: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:cross_polar_discrimination")]
        pub cross_polar_discrimination: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:voltage_standing_wave_ratio")]
        pub voltage_standing_wave_ratio: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:cable_loss")]
        pub cable_loss: Option<f64>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:steerable")]
        pub steerable: Option<bool>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "antenna:mobile")]
        pub mobile: Option<bool>,

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
            return "antenna".to_string();
        }

        /// The version upstream's `extensions/antenna-schema.json` records in its
        /// own `$id`: `.../spec/1.0.0/extensions/antenna-schema`.
        fn version() -> String {
            "1.0.0".to_string()
        }
    }

    #[derive(Debug)]
    pub enum MetadataError {
        Internal(String),
        Serde(serde_json::Error),

        /// A Recording declaring `core:num_channels` other than 1 met the typed
        /// sample API, which has no way to express it in either direction.
        MultiChannelDataset(u64),

        /// A Dataset was asked for as a Rust type that its `core:datatype` does not
        /// describe.
        DatatypeMismatch {
            /// What the Recording says its samples are.
            stored: DataFormat,
            /// What the caller asked to read them as.
            requested: DataFormat,
        },

        /// The samples of a Recording that has no Dataset file were asked for.
        NoDataset,

        /// A Captures segment describes bytes the Dataset does not have.
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
        DatasetPathEscapesDirectory(String),

        /// A Captures segment's bytes are not a whole number of samples.
        PartialSample {
            /// Length of the segment.
            bytes: u64,
            /// The format whose sample width does not divide it.
            datatype: DataFormat,
        },
    }

    impl fmt::Display for MetadataError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                MetadataError::Internal(err_msg) => {
                    write!(f, "Internal error: {}", err_msg)
                }
                MetadataError::Serde(e) => <&serde_json::Error as std::fmt::Display>::fmt(&e, f),
                MetadataError::MultiChannelDataset(channels) => write!(
                    f,
                    "cannot use a typed sample buffer for a Dataset with `core:num_channels` \
                     = {channels}: such a buffer is one channel, and interleaving several \
                     into it would leave `core:datatype` describing something other than \
                     the bytes. The specification recommends SigMF Collections over \
                     `core:num_channels` for multi-channel IQ, for widest application support",
                ),
                MetadataError::DatatypeMismatch { stored, requested } => write!(
                    f,
                    "cannot read a `{stored}` Dataset as `{requested}`: `core:datatype` is \
                     the Recording's own account of what its bytes mean, and reading them \
                     as anything else yields plausible noise rather than an error",
                ),
                MetadataError::NoDataset => f.write_str(
                    "this Recording has no Dataset file: it is either `core:metadata_only`, \
                     or its Metadata file is not named `<basename>.sigmf-meta` and so has no \
                     Dataset that can be named from it, or it has not been written yet",
                ),
                MetadataError::CaptureOutOfBounds {
                    index,
                    start,
                    end,
                    dataset_len,
                } => write!(
                    f,
                    "capture {index} covers bytes {start}..{end} of a Dataset that is \
                     {dataset_len} bytes long. The specification requires `captures` to be \
                     sorted by `core:sample_start` ascending; a Recording whose segments are \
                     not sorted lands here too",
                ),
                MetadataError::DatasetPathEscapesDirectory(name) => write!(
                    f,
                    "`core:dataset` is {name:?}, which is not a plain filename. The \
                     specification says this field \"only includes the filename, not \
                     directory\", and the Dataset \"must be in the same directory as the \
                     .sigmf-meta file\"",
                ),
                MetadataError::PartialSample { bytes, datatype } => write!(
                    f,
                    "a capture holds {bytes} bytes, which is not a whole number of \
                     `{datatype}` samples of {} bytes each",
                    datatype.size(),
                ),
            }
        }
    }

    impl Error for MetadataError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            None
        }
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
        /// use sigmf::sigmf::GlobalMetadata;
        ///
        /// let global = GlobalMetadata::new("cf32_le".parse()?);
        /// assert_eq!(global.datatype.to_string(), "cf32_le");
        /// assert_eq!(global.version, sigmf::sigmf::SIGMF_VERSION);
        /// # Ok::<(), sigmf::sigmf::ParseDataFormatError>(())
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
        /// use sigmf::sigmf::{AntennaGlobal, GlobalMetadata};
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
            match serde_json::to_value(val) {
                Ok(serialized) => match serialized {
                    Value::Object(d) => {
                        let namespace_pattern = T::namespace() + ":";
                        self.other
                            .retain(|k, _| !k.starts_with(namespace_pattern.as_str()));
                        self.other.extend(d);
                        self.declare_extension::<T>();
                        Ok(())
                    }
                    _ => Err(MetadataError::Internal(
                        "unknown serialized message type".to_string(),
                    )),
                },
                Err(e) => Err(MetadataError::Serde(e)),
            }
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
        pub fn delete_extension<T: GlobalExtension>(&mut self) -> Result<(), MetadataError> {
            let namespace_pattern = T::namespace() + ":";
            self.other
                .retain(|k, _| !k.starts_with(namespace_pattern.as_str()));

            if let Some(declared) = &mut self.extensions {
                declared.retain(|e| e.name != T::namespace());
            }
            Ok(())
        }
    }

    /// The byte order of a multi-byte sample.
    ///
    /// Only the multi-byte [`DataType`] variants carry one. A single byte has no
    /// byte order to state, and the specification's datatype grammar reflects that
    /// by omitting the suffix for `i8` and `u8`.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum Endianess {
        /// Most significant byte first, spelled `_be`.
        BigEndian,
        /// Least significant byte first, spelled `_le`.
        LittleEndian,
    }

    impl Endianess {
        /// The suffix this byte order is spelled with in a datatype string.
        fn suffix(self) -> &'static str {
            match self {
                Endianess::BigEndian => "_be",
                Endianess::LittleEndian => "_le",
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
        F32(Endianess),
        /// 64-bit IEEE-754 float, spelled `f64_le` or `f64_be`.
        F64(Endianess),
        /// Signed 32-bit integer, spelled `i32_le` or `i32_be`.
        I32(Endianess),
        /// Signed 16-bit integer, spelled `i16_le` or `i16_be`.
        I16(Endianess),
        /// Unsigned 32-bit integer, spelled `u32_le` or `u32_be`.
        U32(Endianess),
        /// Unsigned 16-bit integer, spelled `u16_le` or `u16_be`.
        U16(Endianess),
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
    /// use sigmf::sigmf::{DataFormat, DataType, Endianess, NumberType};
    ///
    /// let format: DataFormat = "cf32_le".parse()?;
    /// assert_eq!(format.number_type, NumberType::Complex);
    /// assert_eq!(format.data_type, DataType::F32(Endianess::LittleEndian));
    ///
    /// // Complex doubles the width: two f32 components per sample.
    /// assert_eq!(format.size(), 8);
    ///
    /// // Display is the exact inverse of the parse.
    /// assert_eq!(format.to_string(), "cf32_le");
    /// # Ok::<(), sigmf::sigmf::ParseDataFormatError>(())
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
        /// use sigmf::sigmf::{DataFormat, Endianess::{BigEndian, LittleEndian}};
        ///
        /// assert_eq!(DataFormat::of::<Complex<f32>>(LittleEndian).to_string(), "cf32_le");
        /// assert_eq!(DataFormat::of::<i16>(BigEndian).to_string(), "ri16_be");
        ///
        /// // One byte has no byte order, and the datatype does not pretend it does.
        /// assert_eq!(DataFormat::of::<u8>(BigEndian).to_string(), "ru8");
        /// ```
        pub fn of<S: Sample>(endianness: Endianess) -> DataFormat {
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
        /// use sigmf::sigmf::{DataFormat, Endianess};
        ///
        /// assert_eq!("cf32_be".parse::<DataFormat>()?.endianness(), Some(Endianess::BigEndian));
        /// assert_eq!("ri8".parse::<DataFormat>()?.endianness(), None);
        /// # Ok::<(), sigmf::sigmf::ParseDataFormatError>(())
        /// ```
        pub fn endianness(&self) -> Option<Endianess> {
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
        use super::{DataFormat, Endianess};

        /// What [`Sample`](super::Sample) actually provides.
        ///
        /// Unnameable downstream, which makes `Sample` both unimplementable and
        /// free to change: everything here is an implementation detail of the write
        /// path, and the public surface a caller needs is
        /// [`DataFormat::of`](super::DataFormat::of).
        pub trait Sealed {
            /// The `core:datatype` a Dataset of these samples carries.
            fn data_format(endianness: Endianess) -> DataFormat;

            /// Append this sample's bytes, in `endianness`, to `out`.
            ///
            /// Infallible, and writing to a buffer rather than a sink, because the
            /// whole Dataset is assembled in memory before any of it is written —
            /// the samples are already in memory when they arrive, and the checksum
            /// needs a second look at them.
            fn encode(self, endianness: Endianess, out: &mut Vec<u8>);

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
            fn decode(endianness: Endianess, bytes: &[u8]) -> Self;
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
                fn data_format(endianness: Endianess) -> DataFormat {
                    DataFormat {
                        number_type: NumberType::Real,
                        data_type: $data_type(endianness),
                    }
                }

                fn encode(self, endianness: Endianess, out: &mut Vec<u8>) {
                    match endianness {
                        Endianess::LittleEndian => out.extend_from_slice(&self.to_le_bytes()),
                        Endianess::BigEndian => out.extend_from_slice(&self.to_be_bytes()),
                    }
                }

                fn decode(endianness: Endianess, bytes: &[u8]) -> Self {
                    let mut component = [0u8; std::mem::size_of::<$component>()];
                    component.copy_from_slice(bytes);
                    match endianness {
                        Endianess::LittleEndian => Self::from_le_bytes(component),
                        Endianess::BigEndian => Self::from_be_bytes(component),
                    }
                }
            }

            impl Sample for $component {}

            impl private::Sealed for Complex<$component> {
                fn data_format(endianness: Endianess) -> DataFormat {
                    DataFormat {
                        number_type: NumberType::Complex,
                        data_type: $data_type(endianness),
                    }
                }

                fn encode(self, endianness: Endianess, out: &mut Vec<u8>) {
                    // In-phase then quadrature: the interleaving `c` denotes.
                    self.re.encode(endianness, out);
                    self.im.encode(endianness, out);
                }

                fn decode(endianness: Endianess, bytes: &[u8]) -> Self {
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
        MissingEndianess,
        Endianess,
        RedundantEndianess,
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
                ParseDataFormatErrorKind::MissingEndianess => f.write_str(
                    "a multi-byte sample type must state its byte order \
                     with an `_le` or `_be` suffix",
                ),
                ParseDataFormatErrorKind::Endianess => {
                    f.write_str("expected a byte order suffix of `_le` or `_be`")
                }
                ParseDataFormatErrorKind::RedundantEndianess => f.write_str(
                    "a single-byte sample type has no byte order and must not \
                     carry an `_le` or `_be` suffix",
                ),
            }
        }
    }

    impl Error for ParseDataFormatError {}

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
                        return Err(fail(ParseDataFormatErrorKind::RedundantEndianess));
                    }
                    if base == "i8" {
                        DataType::I8
                    } else {
                        DataType::U8
                    }
                }
                "f32" | "f64" | "i32" | "i16" | "u32" | "u16" => {
                    let endianess = match suffix {
                        Some("le") => Endianess::LittleEndian,
                        Some("be") => Endianess::BigEndian,
                        Some(_) => return Err(fail(ParseDataFormatErrorKind::Endianess)),
                        None => return Err(fail(ParseDataFormatErrorKind::MissingEndianess)),
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

#[cfg(test)]
mod test;
