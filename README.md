# Signal Metadata Format (SigMF) in Rust

The [SigMF specification document](https://github.com/sigmf/SigMF/blob/HEAD/sigmf-spec.md).

## Roadmap

### PoC

#### SigMF Metadata

- [x] parse all core fields in metadata
- [ ] support GeoJSON parsing
- [ ] support datetime parsing
- [ ] optional checksum validation
- [ ] reading samples from data file
- [ ] reading multiple channels
- [ ] automatic searching accompanying files (e.g., open data file if metadata is provided)
- [ ] add documentation and doc tests

### Infra

- [ ] run tests on CI
- [ ] configure linters
- [ ] add validation rules to PR
- [ ] configure publishing to crates
- [ ] add license info

### Later

- [ ] SigMF Archive reading
- [ ] support extensions
