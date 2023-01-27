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
FROM --platform=$BUILDPLATFORM rust:latest AS cargo-build
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y cmake musl-tools clang llvm -y

WORKDIR /tmp/helium_gateway
COPY . .

ENV CC_aarch64_unknown_linux_musl=clang
ENV AR_aarch64_unknown_linux_musl=llvm-ar
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-Clink-self-contained=yes -Clinker=rust-lld"

ENV CC_x86_64_unknown_linux_musl=clang
ENV AR_x86_64_unknown_linux_musl=llvm-ar
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-Clink-self-contained=yes -Clinker=rust-lld"

ARG TARGETPLATFORM
RUN \
case "$TARGETPLATFORM" in \
    "linux/arm64") echo aarch64-unknown-linux-musl > rust_target.txt ;; \
    "linux/amd64") echo x86_64-unknown-linux-musl > rust_target.txt ;; \
    *) exit 1 ;; \
esac

RUN rustup target add $(cat rust_target.txt)

RUN cargo build --release --target=$(cat rust_target.txt)
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
COPY --from=cargo-build /tmp/helium_gateway/helium_gateway /usr/local/bin/helium_gateway
RUN mkdir /etc/helium_gateway
COPY config/settings.toml /etc/helium_gateway/settings.toml
CMD ["helium_gateway", "server"]
