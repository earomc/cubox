# syntax=docker/dockerfile:1
# Builds the web version of cubox and serves it with nginx:
#   docker compose up --build    then open http://localhost:8080

FROM rust:1-bookworm AS build
WORKDIR /src

# wasm-opt shrinks and speeds up the wasm, Debian's is too old for it
COPY scripts/install-wasm-opt.sh scripts/
RUN ./scripts/install-wasm-opt.sh

# the pinned nightly with the wasm target, from rust-toolchain.toml
COPY rust-toolchain.toml ./
RUN rustup toolchain install || rustup show

# wasm-bindgen-cli has to match the wasm-bindgen crate in Cargo.lock
COPY Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    version=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p') && \
    cargo install wasm-bindgen-cli --version "$version" --locked

# building SteelMC downloads Minecraft's data, so this step needs network access
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    ./scripts/build-web.sh /dist

FROM nginx:1-alpine
COPY docker/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /dist /usr/share/nginx/html
EXPOSE 80
