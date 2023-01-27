# ------------------------------------------------------------------------------
# Cargo Build Stage
# ------------------------------------------------------------------------------
FROM rust:alpine as cargo-build
RUN apk add --no-cache musl-dev cmake protoc gcompat
WORKDIR /tmp/helium_gateway
COPY . .
ENV PROTOC=/usr/bin/protoc
RUN cargo build --release

# ------------------------------------------------------------------------------
# Final Stage
# ------------------------------------------------------------------------------
FROM alpine:3.17.1
ENV RUST_BACKTRACE=1
ENV GW_UPDATE_ENABLED=false
ENV GW_LISTEN="0.0.0.0:1680"
COPY --from=cargo-build /tmp/helium_gateway/target/release/helium_gateway /usr/local/bin/helium_gateway
RUN mkdir /etc/helium_gateway
COPY config/settings.toml /etc/helium_gateway/settings.toml
CMD ["helium_gateway", "server"]
