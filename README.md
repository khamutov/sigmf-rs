# sigmf

[![crates.io](https://img.shields.io/crates/v/sigmf.svg)](https://crates.io/crates/sigmf)
[![docs.rs](https://docs.rs/sigmf/badge.svg)](https://docs.rs/sigmf)

Read and write [SigMF](https://sigmf.org) recordings in Rust: recorded radio
signals, and the metadata that describes them.

A SigMF **Recording** is two files sharing a basename. `foo.sigmf-data` holds raw
interleaved samples and nothing else — no header, no framing. `foo.sigmf-meta` is a
JSON sidecar saying what those bytes are: sample format, sample rate, centre
frequency, when they were recorded. The point of the format is that the numbers you
need to interpret the bytes travel *with* the bytes, instead of living in a README
or someone's memory.

## Example

```rust,no_run
use sigmf::num_complex::Complex;
use sigmf::{RecordingWriter, SigMF};

let samples = vec![Complex::new(1.0f32, 0.0), Complex::new(0.0, -1.0)];

// The writer asks for the two things only the caller knows — the samples and
// their rate. `core:datatype` is nobody's input: the writer knows the sample type.
let mut writer = RecordingWriter::new(&samples, 32_000.0);
writer.global_mut().recorder = Some("winradio-agent".to_string());

// Writes dsc_watch.sigmf-data and dsc_watch.sigmf-meta.
writer.to_file("dsc_watch")?;

// And back again. The turbofish is checked against `core:datatype`, not assumed.
let reopened = SigMF::from_file("dsc_watch.sigmf-meta")?;
assert_eq!(reopened.samples::<Complex<f32>>()?, samples);
# Ok::<(), sigmf::Error>(())
```

## `core:datatype` is a claim about the bytes

`core:datatype` says how to read every byte of the Dataset. Get it wrong and nothing
errors: `cf32_le` bytes read as `ci16_le` produce plausible noise at the wrong scale,
and a waterfall of plausible noise looks exactly like a waterfall.

So the write path does not accept the claim — it *derives* it. Nothing in
`RecordingWriter`'s surface asks for a datatype: it knows the sample type from the
moment it is handed samples, sets `core:datatype` from that type, and overwrites
whatever the Global held. The field and the bytes cannot disagree, because only one
of them is an input. Reading works the same way in reverse: `samples::<S>()` errors
rather than reinterpret.

The one place a datatype is *stated* rather than derived is
`GlobalMetadata::describing`, for bytes the crate never sees — a Dataset some other
tool wrote, or a `core:metadata_only` document with no Dataset at all. There the
author's word is all there is, and `serde_json::to_string` will happily write
whatever they put in it.

## Specification

This crate implements SigMF **v1.2.6**.

The specification is published at [sigmf.org](https://sigmf.org). There is no spec
document to read in the upstream repository any more — it is *generated*, by
[`docs-generator.py`](https://github.com/sigmf/SigMF/blob/main/docs-generator.py),
from [`sigmf-schema.json`](https://github.com/sigmf/SigMF/blob/main/sigmf-schema.json).
The schema is therefore the specification rather than a description of one.

This crate vendors that schema at `tests/spec/sigmf-schema.json` and uses it as a
test oracle rather than as documentation: fixtures are validated against it before
they are trusted to judge the crate, and metadata the crate writes is validated
against it on the way out.

## Roadmap

### Metadata

- [x] parse all core fields in metadata
- [x] support GeoJSON parsing, including RFC 7946 foreign members
- [x] automatically find the accompanying Dataset, given the Metadata file
- [x] support extensions, including declaring them in `core:extensions`
- [x] documentation and doc-tests
- [ ] support datetime parsing — `core:datetime` is still carried as an
      unvalidated string, where the schema requires RFC 3339 with a `Z` offset
- [ ] optional checksum validation — `core:sha512` is *written*, but nothing
      verifies it on read
- [ ] reading multiple channels — an interleaved multi-channel Dataset is
      refused rather than deinterleaved

### Samples

- [x] write samples to a Dataset, deriving `core:datatype` from the sample type
- [x] read samples from a Dataset, checked against `core:datatype`
- [x] byte ranges of each Captures segment, without reading the samples

### Infra

- [x] run tests on CI
- [x] configure linters
- [x] add validation rules to PR
- [x] configure publishing to crates
- [x] add license info

### Later

- [ ] SigMF Archive (`.sigmf`) reading
- [ ] SigMF Collections (`.sigmf-collection`)

## License

MIT — see [LICENSE](LICENSE).
