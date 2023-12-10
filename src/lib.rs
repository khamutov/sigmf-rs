const SIGMF_ARCHIVE_EXT: &'static str = ".sigmf";
const SIGMF_METADATA_EXT: &'static str = ".sigmf-meta";
const SIGMF_DATASET_EXT: &'static str = ".sigmf-data";
const SIGMF_COLLECTION_EXT: &'static str = ".sigmf-collection";

pub mod sigmf {
    use core::fmt::Debug;
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
        pub fn from_str(s: &str) -> Result<Self, Box<dyn Error>> {
            let metadata = serde_json::from_str(s)?;
            Ok(metadata)
        }

        pub fn to_str(&self) -> Result<String, Box<dyn Error>> {
            let res = serde_json::to_string_pretty(self)?;
            Ok(res)
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
}

#[cfg(test)]
mod test;
