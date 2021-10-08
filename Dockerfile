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
ENV GW_UPDATE_ENABLED=false
ENV GW_LISTEN_ADDR="0.0.0.0:1680"
COPY --from=cargo-build /tmp/helium_gateway/target/x86_64-unknown-linux-musl/release/helium_gateway /usr/local/bin/helium_gateway
RUN mkdir /etc/helium_gateway
CMD ["helium_gateway", "server"]
