# Building from source

You need [rustup](https://rustup.rs). SteelMC's world generator uses nightly Rust features, the right nightly is
pinned in [`rust-toolchain.toml`](../rust-toolchain.toml) and installed automatically.

```sh
git clone https://github.com/earomc/cubox
cd cubox
cargo run --release
```

On Linux, building needs no extra system packages. The first build downloads Minecraft's data for SteelMC.

## Web version

```sh
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # the version of wasm-bindgen in Cargo.lock
./scripts/build-web.sh                                       # builds into dist/
python3 -m http.server --directory dist                      # or any static file server
```

`wasm-opt` from [binaryen](https://github.com/WebAssembly/binaryen) is used when installed, it makes the wasm smaller
and faster. Distributions often ship versions too old for current wasm, `scripts/install-wasm-opt.sh` installs a release.

cubox uses SteelMC with [one commit on top](https://github.com/earomc/SteelMC/tree/cubox-locate-only-26.3) that makes
finding jigsaw structures much faster, until it is upstream. For the browser it uses a copy of simdnbt that allows 32-bit targets
([`vendor/simdnbt`](../vendor/simdnbt/CUBOX.md)).

To try a local build on a phone, serve `dist/` as above and open `http://<your computer's address>:8000` on it.

## Docker

```sh
docker compose up --build
```

builds the web version and serves it at http://localhost:8080 with nginx. The image only contains the static
files, so any static host works too.

## Structure icons

The icons are the PNG files in [`assets/icons`](../assets/icons), named after each structure, 32x32 pixels
with transparency. To change one, replace its file and rebuild. The current ones are placeholders.

## Releasing

1. Bump `version` in `Cargo.toml` and move the changes in [`CHANGELOG.md`](../CHANGELOG.md) under a new
   `## [x.y.z] - date` heading.
2. Commit, then tag and push: `git tag vx.y.z && git push origin main vx.y.z`.

If builds of a release fail, run the workflow by hand on GitHub with its tag: that builds and uploads all assets again.

The [release workflow](../.github/workflows/release.yml) builds all platforms and publishes the release, with the
changelog section as release notes. The [pages workflow](../.github/workflows/pages.yml) deploys the web version to
GitHub Pages on every tag and every push to main: the latest release at the root and main at `/nightly/`.
