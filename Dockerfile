FROM alpine:3.18

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

# Copy source code
COPY . .

# Build the application
RUN cargo build --release

# Create non-root user
RUN addgroup -g 1001 -S turbopix && \
    adduser -u 1001 -S turbopix -G turbopix

# Create data directory and set permissions
RUN mkdir -p data && chown -R turbopix:turbopix /app

# Switch to non-root user
USER turbopix

# Expose port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD wget --no-verbose --tries=1 --spider http://localhost:8080/health || exit 1

# Run the application
CMD ["./target/release/turbo-pix"]