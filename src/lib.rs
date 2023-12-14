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
        pub fn from_str(s: &str, data: &Vec<u8>) -> Result<Self, Box<dyn Error>> {
            let mut metadata: Metadata = serde_json::from_str(s)?;
            metadata.calc_capture_boundaries(data)?;
            Ok(metadata)
        }

        pub fn to_str(&self) -> Result<String, Box<dyn Error>> {
            let res = serde_json::to_string_pretty(self)?;
            Ok(res)
        }

        fn calc_capture_boundaries(&mut self, data: &Vec<u8>) -> Result<(), MetadataError> {
            if self.captures.is_empty() {
                return Ok(());
            }

            let parsed = parse_data_format(self.global.datatype.as_str());
            match parsed {
                Err(_) => Err(MetadataError::Internal(
                    "error parsing datatype".to_string(),
                )),
                Ok((_, data_format)) => {
                    for capture in &mut self.captures {
                        capture.data_format = data_format.clone();
                    }
                    Ok(())
                }
            }?;

            let mut start_byte = 0;
            let last_index = self.captures.len() - 1;
            for index in 0..self.captures.len() {
                let capture = &self.captures[index];
                start_byte += capture.header_bytes.unwrap_or(0);
                start_byte += capture.data_format.size() * capture.sample_start;
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
                    start_byte + next_capture.data_format.size() * next_capture.sample_start as u64
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
        pub datatype: String,

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

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:geolocation")]
        pub geolocation: Option<String>,

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:collection")]
        pub extensions: Option<String>,

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

        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "core:header_bytes")]
        pub header_bytes: Option<u64>,

        #[serde(skip)]
        #[serde(default = "default_capture_boundaries")]
        pub byte_boundaries: (u64, u64),

        #[serde(skip)]
        #[serde(default = "default_capture_data_format")]
        pub data_format: DataFormat,
    }

    fn default_capture_boundaries() -> (u64, u64) {
        (0, 0)
    }

    fn default_capture_data_format() -> DataFormat {
        DataFormat {
            number_type: NumberType::Real,
            data_type: DataType::I8,
        }
    }

    impl CaptureMetadata {}

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

    pub trait GlobalExtension {
        fn namespace() -> String;
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
        pub fn get_extension<T: GlobalExtension + serde::de::DeserializeOwned>(
            &self,
        ) -> Result<T, serde_json::Error> {
            serde_json::from_value(serde_json::json!(self.other))
        }

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
                        Ok(())
                    }
                    _ => Err(MetadataError::Internal(
                        "unknown serialized message type".to_string(),
                    )),
                },
                Err(e) => Err(MetadataError::Serde(e)),
            }
        }

        pub fn delete_extension<T: GlobalExtension + serde::Serialize>(
            &mut self,
        ) -> Result<(), MetadataError> {
            let namespace_pattern = T::namespace() + ":";
            self.other
                .retain(|k, _| !k.starts_with(namespace_pattern.as_str()));
            Ok(())
        }
    }

    impl Default for GlobalMetadata {
        fn default() -> GlobalMetadata {
            GlobalMetadata {
                version: "".to_string(),
                datatype: "".to_string(),
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
    }

    #[derive(Debug, PartialEq, Clone)]
    pub enum Endianess {
        BigEndian,
        LittleEndian,
    }

    #[derive(Debug, PartialEq, Clone)]
    pub enum DataType {
        F32(Endianess),
        F64(Endianess),
        I32(Endianess),
        I16(Endianess),
        U32(Endianess),
        U16(Endianess),
        I8,
        U8,
    }

    impl DataType {
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

    #[derive(Debug, PartialEq, Clone)]
    pub enum NumberType {
        Real,
        Complex,
    }

    #[derive(Debug, PartialEq, Clone)]
    pub struct DataFormat {
        pub number_type: NumberType,
        pub data_type: DataType,
    }

    impl DataFormat {
        pub fn size(&self) -> u64 {
            self.data_type.size()
                * match self.number_type {
                    NumberType::Real => 1,
                    NumberType::Complex => 2,
                }
        }
    }

    enum ParserState {
        F32,
        F64,
        I32,
        I16,
        U32,
        U16,
    }

    impl ParserState {
        fn to_data_type(&self, endianess: Endianess) -> DataType {
            match self {
                ParserState::F32 => DataType::F32(endianess),
                ParserState::F64 => DataType::F64(endianess),
                ParserState::I32 => DataType::I32(endianess),
                ParserState::I16 => DataType::I16(endianess),
                ParserState::U32 => DataType::U32(endianess),
                ParserState::U16 => DataType::U16(endianess),
            }
        }
    }

    fn parse_real(input: &str) -> nom::IResult<&str, NumberType> {
        nom::combinator::map(nom::bytes::complete::tag("r"), |_| NumberType::Real)(input)
    }

    fn parse_complex(input: &str) -> nom::IResult<&str, NumberType> {
        nom::combinator::map(nom::bytes::complete::tag("c"), |_| NumberType::Complex)(input)
    }

    fn parse_type(input: &str) -> nom::IResult<&str, DataType> {
        let (input, data_type) = nom::branch::alt((
            nom::combinator::map(nom::bytes::complete::tag("f32"), |_| ParserState::F32),
            nom::combinator::map(nom::bytes::complete::tag("f64"), |_| ParserState::F64),
            nom::combinator::map(nom::bytes::complete::tag("i32"), |_| ParserState::I32),
            nom::combinator::map(nom::bytes::complete::tag("i16"), |_| ParserState::I16),
            nom::combinator::map(nom::bytes::complete::tag("u32"), |_| ParserState::U32),
            nom::combinator::map(nom::bytes::complete::tag("u16"), |_| ParserState::U16),
        ))(input)?;
        let (input, endianess) = nom::branch::alt((
            nom::combinator::map(nom::bytes::complete::tag("_le"), |_| {
                Endianess::LittleEndian
            }),
            nom::combinator::map(nom::bytes::complete::tag("_be"), |_| Endianess::BigEndian),
        ))(input)?;
        Ok((input, data_type.to_data_type(endianess)))
    }

    fn parse_byte(input: &str) -> nom::IResult<&str, DataType> {
        nom::branch::alt((
            nom::combinator::map(nom::bytes::complete::tag("i8"), |_| DataType::I8),
            nom::combinator::map(nom::bytes::complete::tag("u8"), |_| DataType::U8),
        ))(input)
    }

    fn parse_data_type(input: &str) -> nom::IResult<&str, DataType> {
        nom::branch::alt((parse_type, parse_byte))(input)
    }

    pub fn parse_data_format(input: &str) -> nom::IResult<&str, DataFormat> {
        let (input, number_type) = nom::branch::alt((parse_real, parse_complex))(input)?;
        let (input, data_type) = parse_data_type(input)?;
        Ok((
            input,
            DataFormat {
                number_type: number_type,
                data_type: data_type,
            },
        ))
    }
}

#[cfg(test)]
mod test;
