# simdnbt 0.10.0, patched for cubox

Unchanged copy of [simdnbt](https://github.com/azalea-rs/simdnbt) 0.10.0 from crates.io (MIT
licensed), except that `src/lib.rs` also allows 32-bit targets. cubox's web build is
`wasm32-unknown-unknown`, and simdnbt is compiled in through SteelMC's registry crate, although
cubox never parses NBT. Remove this and the `[patch.crates-io]` entry in `Cargo.toml` once simdnbt
supports 32-bit targets itself.
