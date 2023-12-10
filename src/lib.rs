const SIGMF_ARCHIVE_EXT: &'static str = ".sigmf";
const SIGMF_METADATA_EXT: &'static str = ".sigmf-meta";
const SIGMF_DATASET_EXT: &'static str = ".sigmf-data";
const SIGMF_COLLECTION_EXT: &'static str = ".sigmf-collection";

pub mod sigmf {
    use serde_json::Value;
    use std::collections::BTreeMap as Map;
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

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
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

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct AntennaGlobal {
        #[serde(rename = "antenna:model")]
        pub model: String,
        #[serde(rename = "antenna:type")]
        pub antenna_type: Option<String>,
    }

    impl GlobalMetadata {
        pub fn get_extension<T: serde::de::DeserializeOwned>(
            &self,
        ) -> Result<T, serde_json::Error> {
            serde_json::from_value(serde_json::json!(self.other.clone()))
        }
    }
}

#[cfg(test)]
mod test;
