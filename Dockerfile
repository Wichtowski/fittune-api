# syntax=docker/dockerfile:1

# --- Build ------------------------------------------------------------------
FROM rust:1.94-slim-trixie AS builder
WORKDIR /build

# Compile dependencies against a stub crate first so this layer is cached
# until Cargo.toml / Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && touch src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs src/lib.rs && cargo build --release --locked

# --- Runtime ----------------------------------------------------------------
# Distroless: glibc + libgcc only, no shell, runs as an unprivileged user.
FROM gcr.io/distroless/cc-debian13:nonroot

COPY --from=builder /build/target/release/fittune-api /usr/local/bin/fittune-api

ENV FITTUNE_BIND_ADDR=0.0.0.0:4733 \
    FITTUNE_LOG_FORMAT=json

EXPOSE 4733
USER nonroot

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/fittune-api", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/fittune-api"]
CMD ["serve"]
