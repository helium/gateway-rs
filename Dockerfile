# ------------------------------------------------------------------------------
# Cargo Build Stage
# ------------------------------------------------------------------------------

FROM rust:latest as cargo-build

RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y cmake musl-tools

RUN rustup default nightly
RUN rustup target add x86_64-unknown-linux-musl
# for some reason the proto build script requires this...?
RUN rustup component add rustfmt

WORKDIR /tmp/helium_gateway
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

# ------------------------------------------------------------------------------
# Final Stage
# ------------------------------------------------------------------------------
FROM alpine:latest
COPY --from=cargo-build /tmp/helium_gateway/target/x86_64-unknown-linux-musl/release/helium_gateway /usr/local/bin/helium_gateway
CMD ["helium_gateway", "server"]
