use std::{collections::BTreeMap, error::Error};

use pretty_assertions::assert_eq;
use sigmf::sigmf::*;

#[test]
fn test_open_file() -> Result<(), Box<dyn Error>> {
    let signal = SigMF::from_file("tests/sigmf_test_files/sample.sigmf-meta");
    assert_eq!(
        signal?.metadata.global,
        GlobalMetadata {
            version: "1.0.0".to_string(),
            datatype: "rf32_le".to_string(),
            sample_rate: None,
            num_channels: Some(1),
            sha512: Some("f4984219b318894fa7144519185d1ae81ea721c6113243a52b51e444512a39d74cf41a4cec3c5d000bd7277cc71232c04d7a946717497e18619bdbe94bfeadd6".to_string()),
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
            metadata_only: None, dataset: None, trailing_bytes: None, other: BTreeMap::from([(
                "my_ns:some_prop".to_string(),
                serde_json::value::Value::String("custom_val".to_string())
            )]),
        }
    );
    Ok(())
}
