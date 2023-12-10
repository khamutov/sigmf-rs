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
            num_channels: Some(1),
            sha512: Some("f4984".to_string()),
            other: BTreeMap::from([(
                "my_ns:some_prop".to_string(),
                serde_json::value::Value::String("custom_val".to_string())
            )]),
            ..Default::default()
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
            num_channels: Some(1),
            sha512: Some("f4984".to_string()),
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
            ..Default::default()
        }
    );
    assert_eq!(
        metadata.global.get_extension::<AntennaGlobal>()?,
        AntennaGlobal {
            model: "ARA CSB-16".to_string(),
            antenna_type: Some("dipole".to_string()),
            ..Default::default()
        }
    );
    Ok(())
}

#[test]
fn test_parse_roundtrip() -> Result<(), Box<dyn Error>> {
    let json_data = r#"{
  "global": {
    "core:datatype": "rf32_le",
    "core:version": "1.0.0",
    "core:num_channels": 1,
    "core:sha512": "f4984",
    "my_ns:some_prop": "custom_val"
  },
  "captures": [
    {
      "core:sample_start": 0
    }
  ],
  "annotations": [
    {
      "core:sample_start": 0,
      "core:sample_count": 16
    }
  ]
}"#;
    let metadata = Metadata::from_str(json_data)?;
    assert_eq!(metadata.to_str()?, json_data);
    Ok(())
}

#[test]
fn test_parse_roundtrip_with_extention() -> Result<(), Box<dyn Error>> {
    let json_data = r#"{
  "global": {
    "core:datatype": "rf32_le",
    "core:version": "1.0.0",
    "antenna:model": "ARA CSB-16",
    "antenna:type": "dipole"
  },
  "captures": [],
  "annotations": []
}"#;
    let json_expected = r#"{
  "global": {
    "core:datatype": "rf32_le",
    "core:version": "1.0.0",
    "antenna:model": "new model"
  },
  "captures": [],
  "annotations": []
}"#;
    let mut metadata = Metadata::from_str(json_data)?;

    let mut antenna: AntennaGlobal = metadata.global.get_extension()?;
    antenna.model = "new model".to_string();
    antenna.antenna_type = None;
    metadata.global.set_extension(antenna)?;

    assert_eq!(metadata.to_str()?, json_expected);
    Ok(())
}

#[test]
fn test_parse_roundtrip_with_extention_removal() -> Result<(), Box<dyn Error>> {
    let json_data = r#"{
  "global": {
    "core:datatype": "rf32_le",
    "core:version": "1.0.0",
    "antenna:model": "ARA CSB-16",
    "antenna:type": "dipole"
  },
  "captures": [],
  "annotations": []
}"#;
    let json_expected = r#"{
  "global": {
    "core:datatype": "rf32_le",
    "core:version": "1.0.0"
  },
  "captures": [],
  "annotations": []
}"#;
    let mut metadata = Metadata::from_str(json_data)?;

    metadata.global.delete_extension::<AntennaGlobal>()?;

    assert_eq!(metadata.to_str()?, json_expected);
    Ok(())
}
