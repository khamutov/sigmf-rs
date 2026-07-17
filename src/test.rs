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
    let metadata = Metadata::from_json(json_data)?;
    assert_eq!(
        metadata.global,
        GlobalMetadata {
            version: "1.0.0".to_string(),
            num_channels: Some(1),
            sha512: Some("f4984".to_string()),
            other: BTreeMap::from([(
                "my_ns:some_prop".to_string(),
                serde_json::value::Value::String("custom_val".to_string())
            )]),
            ..GlobalMetadata::describing("rf32_le".parse()?)
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
    let metadata = Metadata::from_json(json_data)?;
    assert_eq!(
        metadata.global,
        GlobalMetadata {
            version: "1.0.0".to_string(),
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
            ..GlobalMetadata::describing("rf32_le".parse()?)
        }
    );
    assert_eq!(
        metadata.global.get_extension::<AntennaGlobal>()?,
        Some(AntennaGlobal {
            model: "ARA CSB-16".to_string(),
            antenna_type: Some("dipole".to_string()),
            ..Default::default()
        })
    );
    Ok(())
}

/// Asking for an extension the Recording does not carry is an answer, not an error.
///
/// This used to report `missing field 'antenna:model'` — a `serde_json::Error`
/// describing a field the caller never mentioned, for a file that is perfectly
/// valid. There was no way to ask "is this extension present?" without matching on
/// an error message.
#[test]
fn absent_extension_reads_as_none() -> Result<(), Box<dyn Error>> {
    let json_data = r#"{
        "global": {
            "core:datatype": "rf32_le",
            "core:version": "1.2.6"
        },
        "captures": [],
        "annotations": []
    }"#;
    let metadata = Metadata::from_json(json_data)?;

    assert_eq!(metadata.global.get_extension::<AntennaGlobal>()?, None);
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
    let metadata = Metadata::from_json(json_data)?;
    assert_eq!(metadata.to_json()?, json_data);
    Ok(())
}

/// Rewriting an extension's fields also declares the namespace.
///
/// The expected output below gained `core:extensions` when `set_extension` learned
/// to declare what it writes. That is worth pausing on: until then this test
/// asserted, byte for byte, output that violates the specification — an undeclared
/// `antenna:model`. The suite was not merely blind to the defect, it required it.
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
    "core:extensions": [
      {
        "name": "antenna",
        "version": "1.0.0",
        "optional": true
      }
    ],
    "antenna:model": "new model"
  },
  "captures": [],
  "annotations": []
}"#;
    let mut metadata = Metadata::from_json(json_data)?;

    let mut antenna: AntennaGlobal = metadata
        .global
        .get_extension()?
        .expect("the fixture carries antenna keys");
    antenna.model = "new model".to_string();
    antenna.antenna_type = None;
    metadata.global.set_extension(antenna)?;

    assert_eq!(metadata.to_json()?, json_expected);
    Ok(())
}

/// Declaring a namespace twice replaces the declaration rather than duplicating it.
#[test]
fn setting_an_extension_twice_declares_it_once() -> Result<(), Box<dyn Error>> {
    let mut global = GlobalMetadata::describing("rf32_le".parse()?);

    for model in ["ARA CSB-16", "Wellbrook ALA1530"] {
        global.set_extension(AntennaGlobal {
            model: model.to_string(),
            ..Default::default()
        })?;
    }

    assert_eq!(
        global.extensions,
        Some(vec![Extension {
            name: "antenna".to_string(),
            version: "1.0.0".to_string(),
            optional: true,
        }])
    );
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
    let mut metadata = Metadata::from_json(json_data)?;

    metadata.global.delete_extension::<AntennaGlobal>();

    assert_eq!(metadata.to_json()?, json_expected);
    Ok(())
}

/// Deleting an extension retracts its declaration, and leaves other declarations
/// alone.
///
/// A stale declaration is not cosmetic. A non-`optional` one instructs a reader to
/// refuse a Recording it cannot support — so leaving one behind for data that is no
/// longer there tells readers to reject a file that is now perfectly parseable.
#[test]
fn deleting_an_extension_undeclares_only_that_extension() -> Result<(), Box<dyn Error>> {
    let json_data = r#"{
        "global": {
            "core:datatype": "rf32_le",
            "core:version": "1.2.6",
            "core:extensions": [
                { "name": "antenna", "version": "1.0.0", "optional": true },
                { "name": "capture_details", "version": "1.0.0", "optional": false }
            ],
            "antenna:model": "ARA CSB-16",
            "capture_details:emitter": "coast-station"
        },
        "captures": [],
        "annotations": []
    }"#;
    let mut metadata = Metadata::from_json(json_data)?;

    metadata.global.delete_extension::<AntennaGlobal>();

    assert_eq!(
        metadata.global.extensions,
        Some(vec![Extension {
            name: "capture_details".to_string(),
            version: "1.0.0".to_string(),
            optional: false,
        }]),
        "only the deleted namespace loses its declaration"
    );
    assert!(
        metadata
            .global
            .other
            .contains_key("capture_details:emitter"),
        "the other namespace's data is untouched"
    );
    Ok(())
}

mod data_format {
    //! `core:datatype` is a claim about the bytes, so the parse is the crate's
    //! narrowest and most load-bearing boundary.

    use pretty_assertions::assert_eq;

    use crate::sigmf::*;

    /// Every format the specification permits: two number types across six
    /// multi-byte sample types in both byte orders, plus the two single-byte types
    /// that have no byte order.
    fn every_format() -> Vec<DataFormat> {
        let mut out = Vec::new();
        for number_type in [NumberType::Real, NumberType::Complex] {
            for data_type in [
                DataType::F32(Endianness::LittleEndian),
                DataType::F32(Endianness::BigEndian),
                DataType::F64(Endianness::LittleEndian),
                DataType::F64(Endianness::BigEndian),
                DataType::I32(Endianness::LittleEndian),
                DataType::I32(Endianness::BigEndian),
                DataType::I16(Endianness::LittleEndian),
                DataType::I16(Endianness::BigEndian),
                DataType::U32(Endianness::LittleEndian),
                DataType::U32(Endianness::BigEndian),
                DataType::U16(Endianness::LittleEndian),
                DataType::U16(Endianness::BigEndian),
                DataType::I8,
                DataType::U8,
            ] {
                out.push(DataFormat {
                    number_type,
                    data_type,
                });
            }
        }
        out
    }

    #[test]
    fn parses_the_spelling_the_spec_gives() {
        assert_eq!(
            "cf32_le".parse::<DataFormat>().unwrap(),
            DataFormat {
                number_type: NumberType::Complex,
                data_type: DataType::F32(Endianness::LittleEndian),
            }
        );
        assert_eq!(
            "ru16_be".parse::<DataFormat>().unwrap(),
            DataFormat {
                number_type: NumberType::Real,
                data_type: DataType::U16(Endianness::BigEndian),
            }
        );
        assert_eq!(
            "cu8".parse::<DataFormat>().unwrap(),
            DataFormat {
                number_type: NumberType::Complex,
                data_type: DataType::U8,
            }
        );
    }

    /// `Display` is the exact inverse of the parse, for every representable value.
    ///
    /// Both directions carry a hand-written table of the spec's spellings. This is
    /// what stops them drifting apart — and what makes it safe for the write path to
    /// derive `core:datatype` rather than accept it.
    #[test]
    fn display_round_trips_through_parse_for_every_format() {
        for format in every_format() {
            let spelled = format.to_string();
            assert_eq!(
                spelled.parse::<DataFormat>().unwrap(),
                format,
                "{spelled} did not survive a Display -> parse round-trip"
            );
        }
        assert_eq!(every_format().len(), 28);
    }

    /// The parse consumes its whole input.
    ///
    /// The schema's own regex does not: `^(c|r)(f32|...)(_le|_be)?` carries no `$`
    /// anchor, so `cf32_le_GARBAGE` satisfies it by matching a prefix. The oracle
    /// cannot catch this class of input, so the crate must.
    #[test]
    fn rejects_trailing_garbage() {
        for input in ["cf32_le_GARBAGE", "cf32_lex", "rf32_xx", "cf32_le!!!"] {
            assert!(
                input.parse::<DataFormat>().is_err(),
                "{input} must not parse: everything after the sample type must be \
                 exactly a byte-order suffix"
            );
        }
    }

    /// A multi-byte type must state its byte order; guessing would byte-swap.
    #[test]
    fn rejects_a_multi_byte_type_without_a_byte_order() {
        for input in ["cf32", "rf64", "ci16", "ru32"] {
            assert!(
                input.parse::<DataFormat>().is_err(),
                "{input} must not parse"
            );
        }
    }

    /// A single byte has no byte order to state.
    ///
    /// Accepting `ri8_le` would force `Display` to emit `ri8`, so opening a file and
    /// writing it back would silently rewrite a required field.
    #[test]
    fn rejects_a_byte_order_on_a_single_byte_type() {
        for input in ["ri8_le", "cu8_be"] {
            assert!(
                input.parse::<DataFormat>().is_err(),
                "{input} must not parse"
            );
        }
    }

    /// The specification permits no 64-bit integers, and the empty string is not a
    /// datatype — the value `GlobalMetadata::default()` used to supply before it was
    /// replaced by a constructor that demands a real one.
    #[test]
    fn rejects_types_outside_the_spec() {
        for input in ["ci64_le", "ru64_le", "", "c", "cf", "xf32_le", "CF32_LE"] {
            assert!(
                input.parse::<DataFormat>().is_err(),
                "{input:?} must not parse"
            );
        }
    }

    /// A rejected datatype fails the whole file, whether or not it has captures.
    ///
    /// Validation used to live in `calc_capture_boundaries`, which returns early on
    /// an empty `captures` array — so a bogus datatype was caught or ignored
    /// depending on unrelated content. It is now the deserializer's job.
    #[test]
    fn a_bogus_datatype_fails_a_capture_less_file() {
        let json_data = r#"{
            "global": {
                "core:datatype": "totally-not-a-datatype",
                "core:version": "1.2.6"
            },
            "captures": [],
            "annotations": []
        }"#;
        let err = Metadata::from_json(json_data)
            .expect_err("a file whose datatype cannot describe any bytes must not open");
        assert!(
            err.to_string().contains("invalid SigMF datatype"),
            "the error must name the real problem, got: {err}"
        );
    }

    /// The error says what was wrong, not merely that something was.
    #[test]
    fn errors_name_the_input_and_the_expectation() {
        let err = "ci64_le".parse::<DataFormat>().unwrap_err().to_string();
        assert!(err.contains("ci64_le"), "got: {err}");
        assert!(
            err.contains("i8"),
            "the message should list what is permitted: {err}"
        );

        let err = "cf32".parse::<DataFormat>().unwrap_err().to_string();
        assert!(err.contains("_le"), "got: {err}");
    }

    /// Complex samples are two components wide.
    #[test]
    fn size_counts_both_components_of_a_complex_sample() {
        assert_eq!("cf32_le".parse::<DataFormat>().unwrap().size(), 8);
        assert_eq!("rf32_le".parse::<DataFormat>().unwrap().size(), 4);
        assert_eq!("ci16_le".parse::<DataFormat>().unwrap().size(), 4);
        assert_eq!("ru8".parse::<DataFormat>().unwrap().size(), 1);
    }
}

/// Where each Captures segment's samples sit in the Dataset.
///
/// These tests once fed `Metadata::from_str` an 8000-byte buffer of zeroes and read
/// the answer off a field. The buffer was never inspected — only its length was —
/// and passing `8000` says so, which is the whole of the change to the six tests
/// that predate this module's docs. Every expected byte range below is the number
/// they asserted before.
#[cfg(test)]
mod capture_tests {
    use std::error::Error;

    use crate::sigmf::{Metadata, MetadataError};

    #[test]
    fn test_boundary_one() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "rf32_le",
                "core:version": "1.0.0",
                "core:num_channels": 1
            },
            "captures": [
                {
                    "core:sample_start": 0
                }
            ],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;

        assert_eq!(metadata.capture_boundaries(8000)?[0], 0..8000);
        Ok(())
    }

    #[test]
    fn test_boundary_multiple() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0",
                "core:num_channels": 1
            },
            "captures": [
                {
                    "core:sample_start": 0
                },
                {
                    "core:sample_start": 500
                }
            ],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;

        assert_eq!(metadata.capture_boundaries(8000)?[1], 4000..8000);
        Ok(())
    }

    #[test]
    fn test_boundary_with_header() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0",
                "core:num_channels": 1
            },
            "captures": [
                {
                    "core:sample_start": 0,
                    "core:header_bytes": 6
                },
                {
                    "core:sample_start": 500,
                    "core:header_bytes": 12
                }
            ],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;
        let boundaries = metadata.capture_boundaries(8000)?;

        assert_eq!(boundaries[0], 6..4006);
        assert_eq!(boundaries[1], 4018..8000);
        Ok(())
    }

    #[test]
    fn test_boundary_with_trailing_first_chunk() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0",
                "core:num_channels": 1,
                "core:trailing_bytes": 50
            },
            "captures": [
                {
                    "core:sample_start": 0
                }
            ],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;

        assert_eq!(metadata.capture_boundaries(8000)?[0], 0..7950);
        Ok(())
    }

    #[test]
    fn test_boundary_with_trailing_last_chunk() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0",
                "core:num_channels": 1,
                "core:trailing_bytes": 50
            },
            "captures": [
                {
                    "core:sample_start": 0
                },
                {
                    "core:sample_start": 500
                }
            ],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;
        let boundaries = metadata.capture_boundaries(8000)?;

        assert_eq!(boundaries[0], 0..4000);
        assert_eq!(boundaries[1], 4000..7950);
        Ok(())
    }

    #[test]
    fn test_boundary_with_trailing_and_header_last_chunk() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0",
                "core:num_channels": 1,
                "core:trailing_bytes": 50
            },
            "captures": [
                {
                    "core:sample_start": 0,
                    "core:header_bytes": 6
                },
                {
                    "core:sample_start": 500,
                    "core:header_bytes": 12
                }
            ],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;
        let boundaries = metadata.capture_boundaries(8000)?;

        assert_eq!(boundaries[0], 6..4006);
        assert_eq!(boundaries[1], 4018..7950);
        Ok(())
    }

    /// Each segment's offset is absolute, and does not accumulate down the array.
    ///
    /// Regression test. The boundary code used to add every segment's
    /// `core:sample_start` to a running offset, so segment `k` began
    /// `sample_size * sum(sample_start[0..k])` bytes late. Two segments whose first
    /// starts at sample 0 add nothing to that sum, which is why the six tests above
    /// — all of them — agree with both the correct arithmetic and the broken kind.
    /// Three segments do not.
    #[test]
    fn a_segments_offset_is_absolute_not_a_running_total() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0",
                "core:num_channels": 1
            },
            "captures": [
                { "core:sample_start": 0 },
                { "core:sample_start": 100 },
                { "core:sample_start": 500 }
            ],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;
        let boundaries = metadata.capture_boundaries(8000)?;

        // 8 bytes per cf32_le sample: samples 0..100, 100..500, 500..1000.
        assert_eq!(boundaries[0], 0..800);
        assert_eq!(boundaries[1], 800..4000);
        assert_eq!(boundaries[2], 4000..8000);
        Ok(())
    }

    /// An empty `captures` array is one implicit segment, not zero segments.
    ///
    /// The specification: `"captures": []` implies
    /// `"captures": [{"core:sample_start": 0}]`. Answering with no boundaries would
    /// say the Dataset holds no samples, which is a different claim than the file
    /// makes.
    #[test]
    fn an_empty_captures_array_covers_the_whole_dataset() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0"
            },
            "captures": [],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;

        assert_eq!(metadata.capture_boundaries(8000)?, vec![0..8000]);
        Ok(())
    }

    /// A segment cannot describe bytes the Dataset does not have.
    ///
    /// Left unchecked this is not merely a wrong number: `samples()` slices the
    /// Dataset by these ranges, so an out-of-range end is a panic on a malformed
    /// file.
    #[test]
    fn a_segment_reaching_past_the_dataset_is_refused() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0"
            },
            "captures": [
                { "core:sample_start": 0 },
                { "core:sample_start": 5000 }
            ],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;

        // Sample 5000 of a cf32_le Dataset is byte 40000; the Dataset has 8000.
        let err = metadata
            .capture_boundaries(8000)
            .expect_err("a segment beyond the Dataset must not be answered with a byte range");
        assert!(
            matches!(err, MetadataError::CaptureOutOfBounds { index: 0, .. }),
            "got: {err:?}"
        );
        Ok(())
    }

    /// `core:trailing_bytes` cannot exceed the Dataset it trails.
    #[test]
    fn trailing_bytes_larger_than_the_dataset_are_refused() -> Result<(), Box<dyn Error>> {
        let json_data = r#"{
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.0.0",
                "core:trailing_bytes": 9000
            },
            "captures": [{ "core:sample_start": 0 }],
            "annotations": []
        }"#;
        let metadata = Metadata::from_json(json_data)?;

        let err = metadata.capture_boundaries(8000).expect_err(
            "trailing bytes past the start of the Dataset must not subtract to a range",
        );
        assert!(err.to_string().contains("trailing_bytes"), "got: {err}");
        Ok(())
    }
}
