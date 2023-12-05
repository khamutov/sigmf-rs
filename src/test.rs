use std::{collections::BTreeMap, error::Error};

use crate::sigmf::*;

#[test]
fn test_open_file() -> Result<(), Box<dyn Error>> {
    let json_data = r#"
    {
        "global": {
            "core:datatype": "rf32_le",
            "core:num_channels": 1,
            "core:sha512": "f4984219b318894fa7144519185d1ae81ea721c6113243a52b51e444512a39d74cf41a4cec3c5d000bd7277cc71232c04d7a946717497e18619bdbe94bfeadd6",
            "core:version": "1.0.0",
            "my_ns:some_prop": "custom_val"
        },
        "captures": [
            {
                "core:sample_start": 0
            }
        ],
        "annotations": [
            {
                "core:sample_count": 16,
                "core:sample_start": 0
            }
        ]
    }"#;
    let metadata = Metadata::from_str(json_data)?;
    assert_eq!(
        metadata.global,
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
            metadata_only: None,
            dataset: None,
            trailing_bytes: None,
            other: BTreeMap::from([("my_ns:some_prop".to_string(), serde_json::value::Value::String("custom_val".to_string()) )]),
        }
    );
    Ok(())
}
