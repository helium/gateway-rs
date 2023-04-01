# ==============================================================================
# This Docker file is designed support multi-architecture[1] images.
#
# How you build depends on your use case. If you only want to build
# for the architecture you're invoking docker from (host arch):
#
#     docker build .
#
# However, if you want to build for another architecture or multiple
# architectures, use buildx[2]:
#
#     docker buildx build --platform linux/arm64,linux/amd64 .
#
# Adding support for additional architectures requires editing the
# `case "$TARGETPLATFORM" in` in the build stage (and likely quite a
# bit of googling).
#
# 1: https://www.docker.com/blog/how-to-rapidly-build-multi-architecture-images-with-buildx
# 2: https://docs.docker.com/build/install-buildx
# ==============================================================================


# ------------------------------------------------------------------------------
# Cargo Build Stage
# ------------------------------------------------------------------------------
FROM rust:alpine3.17 AS cargo-build
ARG FEATURES
RUN apk add --no-cache --update \
    clang15-libclang \
    cmake \
    g++ \
    gcc \
    libc-dev \
    musl-dev \
    protobuf \
    tpm2-tss-dev

WORKDIR /tmp/helium_gateway
COPY . .

ENV CC=gcc CXX=g++ CFLAGS="-U__sun__" RUSTFLAGS="-C target-feature=-crt-static"

# TMP build fail when cross compiling, so we need to use QEMU when
# building for not-host architectures. But QUEMU builds fail in CI due
# to OOMing on cargo registry updating. Therefore, we will need to
# compile with nightly until cargo's sparse registry stabilizes.
RUN rustup toolchain install nightly
ENV CARGO_UNSTABLE_SPARSE_REGISTRY=true

RUN cargo +nightly build --release --features=tpm
RUN mv target/release/helium_gateway .


# ------------------------------------------------------------------------------
# Final Stage
#
# Run steps run in a VM based on the target architecture
# Produces image for target architecture
# ------------------------------------------------------------------------------
FROM alpine:3.17.1
ENV RUST_BACKTRACE=1
ENV GW_LISTEN="127.0.0.1:1680"
RUN apk add --no-cache --update \
    libstdc++ \
    tpm2-tss-esys \
    tpm2-tss-fapi \
    tpm2-tss-mu \
    tpm2-tss-rc \
    tpm2-tss-tcti-device

COPY --from=cargo-build /tmp/helium_gateway/helium_gateway /usr/local/bin/helium_gateway
RUN mkdir /etc/helium_gateway
COPY config/settings.toml /etc/helium_gateway/settings.toml
CMD ["helium_gateway", "server"]
