const SIGMF_ARCHIVE_EXT: &'static str = ".sigmf";
const SIGMF_METADATA_EXT: &'static str = ".sigmf-meta";
const SIGMF_DATASET_EXT: &'static str = ".sigmf-data";
const SIGMF_COLLECTION_EXT: &'static str = ".sigmf-collection";

pub mod sigmf {
    use core::fmt::Debug;
    use serde_json::de::Read;
    use serde_json::Value;
    use std::collections::BTreeMap as Map;
    use std::fmt;
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
        pub fn from_file<T: AsRef<Path>>(path: T) -> Result<Self, Box<dyn Error>> {
            let metadata_file = fs::File::open(path)?;
            let metadata = serde_json::from_reader(metadata_file)?;
            Ok(Self {
                metadata: metadata,
                datafile: None,
            })
        }
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct Metadata {
        pub global: GlobalMetadata,
        pub captures: Vec<CaptureMetadata>,
        pub annotations: Vec<AnnotationMetadata>,
    }

    impl Metadata {
        pub fn from_str(s: &str, data: &[u8]) -> Result<Self, Box<dyn Error>> {
            let mut metadata: Metadata = serde_json::from_str(s)?;
            metadata.calc_capture_boundaries(data)?;
            Ok(metadata)
        }

        pub fn to_str(&self) -> Result<String, Box<dyn Error>> {
            let res = serde_json::to_string_pretty(self)?;
            Ok(res)
        }

        fn calc_capture_boundaries(&mut self, data: &[u8]) -> Result<(), MetadataError> {
            if self.captures.is_empty() {
                return Ok(());
            }

            // Every capture in a recording shares one sample format: the global
            // `core:datatype`, already parsed at the file boundary.
            let sample_size = self.global.datatype.size();

            let mut start_byte = 0;
            let last_index = self.captures.len() - 1;
            for index in 0..self.captures.len() {
                let capture = &self.captures[index];
                start_byte += capture.header_bytes.unwrap_or(0);
                start_byte += sample_size * capture.sample_start;
                let end_byte = if index == last_index {
                    let last_data_byte =
                        data.len() as i64 - self.global.trailing_bytes.unwrap_or(0) as i64;
                    if last_data_byte < 0 {
                        return Err(MetadataError::Internal(format!(
                            "Trailing offset {} is bigger than data size {}",
                            self.global.trailing_bytes.unwrap_or(0),
                            data.len(),
                        )));
                    }
                    last_data_byte as u64
                } else {
                    let next_capture = &self.captures[index + 1];
                    start_byte + sample_size * next_capture.sample_start
                };

                if start_byte > end_byte {
                    return Err(MetadataError::Internal(format!(
                        "Starting offset [{}] for capture {} is bigger than data file size [{}]",
                        start_byte, index, end_byte,
                    )));
                }
                self.captures[index].byte_boundaries = (start_byte, end_byte);
            }
            Ok(())
        }
    }

    #[derive(Debug, Deserialize, Serialize)]
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

        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    impl PartialEq for GlobalMetadata {
        fn eq(&self, other: &Self) -> bool {
            self.datatype == other.datatype
                && self.sample_rate == other.sample_rate
                && self.version == other.version
                && self.num_channels == other.num_channels
                && self.sha512 == other.sha512
                && self.offset == other.offset
                && self.description == other.description
                && self.author == other.author
                && self.meta_doi == other.meta_doi
                && self.data_doi == other.data_doi
                && self.recorder == other.recorder
                && self.license == other.license
                && self.hw == other.hw
                && self.geolocation == other.geolocation
                && self.extensions == other.extensions
                && self.collection == other.collection
                && self.metadata_only == other.metadata_only
                && self.dataset == other.dataset
                && self.trailing_bytes == other.trailing_bytes
                && self.other == other.other
        }
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

        #[serde(skip)]
        #[serde(default = "default_capture_boundaries")]
        pub byte_boundaries: (u64, u64),
    }

    fn default_capture_boundaries() -> (u64, u64) {
        (0, 0)
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
    }

    impl fmt::Display for MetadataError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                MetadataError::Internal(err_msg) => {
                    write!(f, "Internal error: {}", err_msg)
                }
                MetadataError::Serde(e) => <&serde_json::Error as std::fmt::Display>::fmt(&e, f),
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
    }

    impl fmt::Display for DataFormat {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}{}", self.number_type, self.data_type)
        }
    }

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
