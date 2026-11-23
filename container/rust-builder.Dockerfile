FROM rust:alpine3.22

RUN apk add --no-cache \
    build-base \
    ca-certificates \
    cmake \
    curl \
    perl \
    pkgconf \
    tar \
    zstd

WORKDIR /app
