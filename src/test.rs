use pretty_assertions::assert_eq;
use std::{collections::BTreeMap, error::Error};

use crate::sigmf::*;

#[test]
fn test_parse_metadata() -> Result<(), Box<dyn Error>> {
    let json_data = r#"
    {
        "global": {
            "core:datatype": "rf32_le",
            "core:num_channels": 1,
            "core:sha512": "f4984",
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
            sha512: Some("f4984".to_string()),
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
            other: BTreeMap::from([(
                "my_ns:some_prop".to_string(),
                serde_json::value::Value::String("custom_val".to_string())
            )]),
        }
    );
    Ok(())
}

#[test]
fn test_parse_metadata_with_antenna() -> Result<(), Box<dyn Error>> {
    let json_data = r#"
    {
        "global": {
            "core:datatype": "rf32_le",
            "core:num_channels": 1,
            "core:sha512": "f4984",
            "core:version": "1.0.0",
            "antenna:model": "ARA CSB-16",
            "antenna:type": "dipole"
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
            sha512: Some("f4984".to_string()),
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
            other: BTreeMap::from([
                (
                    "antenna:model".to_string(),
                    serde_json::value::Value::String("ARA CSB-16".to_string())
                ),
                (
                    "antenna:type".to_string(),
                    serde_json::value::Value::String("dipole".to_string())
                )
            ]),
        }
    );
    assert_eq!(
        metadata.global.get_extension::<AntennaGlobal>()?,
        AntennaGlobal {
            model: "ARA CSB-16".to_string(),
            antenna_type: Some("dipole".to_string())
        }
    );
    Ok(())
}
