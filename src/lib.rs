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
    }

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct GlobalMetadata {
        #[serde(rename = "core:datatype")]
        pub datatype: String,
        #[serde(rename = "core:sample_rate")]
        pub sample_rate: Option<f64>,
        #[serde(rename = "core:version")]
        pub version: String,
        #[serde(rename = "core:num_channels")]
        pub num_channels: Option<u64>,
        #[serde(rename = "core:sha512")]
        pub sha512: Option<String>,
        #[serde(rename = "core:offset")]
        pub offset: Option<u64>,
        #[serde(rename = "core:description")]
        pub description: Option<String>,
        #[serde(rename = "core:author")]
        pub author: Option<String>,
        #[serde(rename = "core:meta_doi")]
        pub meta_doi: Option<String>,
        #[serde(rename = "core:data_doi")]
        pub data_doi: Option<String>,
        #[serde(rename = "core:recorder")]
        pub recorder: Option<String>,
        #[serde(rename = "core:license")]
        pub license: Option<String>,
        #[serde(rename = "core:hw")]
        pub hw: Option<String>,
        #[serde(rename = "core:geolocation")]
        pub geolocation: Option<String>,
        #[serde(rename = "core:collection")]
        pub extensions: Option<String>,
        #[serde(rename = "core:collection")]
        pub collection: Option<String>,
        #[serde(rename = "core:metadata_only")]
        pub metadata_only: Option<bool>,
        #[serde(rename = "core:dataset")]
        pub dataset: Option<String>,
        #[serde(rename = "core:trailing_bytes")]
        pub trailing_bytes: Option<u64>,
        #[serde(flatten)]
        pub other: Map<String, Value>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct CaptureMetadata {
        start: String,
        center_frequency: f64,
        sample_rate: f64,
    }
}

#[cfg(test)]
mod test;
