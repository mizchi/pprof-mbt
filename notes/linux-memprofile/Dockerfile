# End-to-end verification for `moon-pprof memprofile-native` on Linux.
#
# The host is darwin/arm64; this image is linux/aarch64 (matches Docker
# Desktop's VM). We bind-mount the repo at /work and use a Docker
# volume for /work/target so cargo doesn't fight the host's macOS
# build artifacts.
#
# Build:
#   docker build -t moon-pprof-linux -f notes/linux-memprofile/Dockerfile .
# Run:
#   bash notes/linux-memprofile/run.sh

FROM rust:1.95-bookworm

# moon's cc backend uses the system `cc` (gcc on Debian) and links
# libdl/libpthread/libm. protoc is needed at cargo-build time by the
# prost-build step in firefox-to-pprof. binutils provides `addr2line`
# which dladdr uses on some glibc setups.
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential \
      ca-certificates \
      curl \
      protobuf-compiler \
      binutils \
      libc6-dev \
    && rm -rf /var/lib/apt/lists/*

ENV MOON_HOME=/root/.moon
ENV PATH=${MOON_HOME}/bin:${PATH}

# Pin to whatever the host has on Darwin so behaviour matches. We pull
# `latest` here; for stricter reproducibility pass --build-arg
# MOONBIT_VERSION=<sha> on `docker build`.
ARG MOONBIT_VERSION=latest
RUN curl -fsSL https://cli.moonbitlang.com/install/unix.sh | bash -s -- ${MOONBIT_VERSION}

RUN moon version --all

WORKDIR /work
