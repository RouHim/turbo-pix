# # # # # # # # # # # # # # # # # # # #
# Builder
# # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
FROM docker.io/alpine:3.18 as builder

# Install Rust and build dependencies
RUN apk add --no-cache \
    rust \
    cargo \
    musl-dev \
    sqlite-dev \
    pkgconfig \
    build-base \
    ca-certificates

# Create app directory
WORKDIR /app

# Copy dependency files first for better caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy source code
COPY src ./src
COPY static ./static

# Build the application
RUN cargo build --release

# Create an empty directory that will be used in the final image
RUN mkdir "/empty_dir"

# # # # # # # # # # # # # # # # # # # #
# Run image
# # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
FROM scratch

ENV USER "1000"
ENV DATA_FOLDER "/data"
ENV STATIC_FOLDER "/static"
ENV RUST_LOG "info"

# For performance reasons write data to docker volume instead of containers writeable fs layer
VOLUME $DATA_FOLDER

# Copy the empty directory as data and temp folder
COPY --chown=$USER:$USER --from=builder /empty_dir $DATA_FOLDER
COPY --chown=$USER:$USER --from=builder /empty_dir /tmp

# Copy the built application from the build image to the run-image
COPY --chown=$USER:$USER --from=builder /app/target/release/turbo-pix /turbo-pix

# Copy static files
COPY --chown=$USER:$USER --from=builder /app/static $STATIC_FOLDER

EXPOSE 18473
USER $USER

ENTRYPOINT ["/turbo-pix"]