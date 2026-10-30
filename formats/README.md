# formats

On-disk and wire format specifications and the freeze corpus (`corpus/`). Every `*.golden` file needs a
`*.golden.toml` with `format` and `version`. The corpus only grows. Must not: contain code, except the
standard-library Python generators of independent golden files (`generate_*.py`), which never call a Rust
codec.
