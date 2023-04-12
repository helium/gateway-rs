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
#
# Runs on native host architecture
# Cross compiles for target architecture
# ------------------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM rust:alpine3.17 AS cargo-build
# RUN apk add --no-cache --update clang15-libclang cmake musl-dev protobuf
RUN apk add --no-cache --update \
    clang15-libclang \
    cmake \
    g++ \
    gcc \
    libc-dev \
    musl-dev \
    protobuf \
    tpm2-tss-dev

ARG TARGETPLATFORM
ARG BUILDPLATFORM

# We will never enable TPM on anything other than x86
# RUN if [[ "$TARGETPLATFORM" == "linux/amd64" ]] ; then apk add g++ gcc libc-dev tpm2-tss-dev ; fi

WORKDIR /tmp/helium_gateway
COPY . .

ENV CC_aarch64_unknown_linux_musl=clang
ENV AR_aarch64_unknown_linux_musl=llvm-ar
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-Clink-self-contained=yes -Clinker=rust-lld"

# ENV CC=gcc CXX=g++
# ENV CC_x86_64_unknown_linux_musl=gcc
# ENV AR_x86_64_unknown_linux_musl=llvm-ar
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-Ctarget-feature=-crt-static"

# ENV CC=gcc CXX=g++ CFLAGS="-U__sun__" RUSTFLAGS="-C target-feature=-crt-static"

RUN \
case "$BUILDPLATFORM $TARGETPLATFORM" in \
    "linux/amd64 linux/arm64") \
        echo "--target=aarch64-unknown-linux-musl" > rust_target.txt ; \
        rustup target add aarch64-unknown-linux-musl ; \
        echo > rust_features.txt \
        ;; \
    "linux/amd64 linux/amd64") \
        echo > rust_target.txt ; \
        echo "--no-default-features --features=tpm" > rust_features.txt ; \
        ;; \
    *) \
        exit 1 \
        ;; \
esac

RUN cargo build --release $(cat rust_target.txt) $(cat rust_features.txt)
RUN mv target/$(cat rust_target.txt)/release/helium_gateway .


# ------------------------------------------------------------------------------
# Final Stage
#
# Run steps run in a VM based on the target architecture
# Produces image for target architecture
# ------------------------------------------------------------------------------
FROM alpine:3.17.1
ENV RUST_BACKTRACE=1
ENV GW_LISTEN="0.0.0.0:1680"
ARG TARGETPLATFORM

# We will never enable TPM on anything other than x86
RUN \
if [ "$TARGETPLATFORM" = "linux/amd64" ]; \
    then apk add --no-cache --update \
    libstdc++ \
    tpm2-tss-esys \
    tpm2-tss-fapi \
    tpm2-tss-mu \
    tpm2-tss-rc \
    tpm2-tss-tcti-device ; \
fi

COPY --from=cargo-build /tmp/helium_gateway/helium_gateway /usr/local/bin/helium_gateway
RUN mkdir /etc/helium_gateway
COPY config/settings.toml /etc/helium_gateway/settings.toml
CMD ["helium_gateway", "server"]
