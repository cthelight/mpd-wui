FROM rust:1.98-slim AS builder

RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY web ./web

RUN cargo build --release --locked --target x86_64-unknown-linux-musl -p mpd-wui

# musl is statically linked into the binary, so it runs on scratch with no
# base filesystem: the image is the binary and nothing else.
FROM scratch

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/mpd-wui /mpd-wui

# scratch has no /etc/passwd, so use the numeric "nobody" identity.
USER 65532:65532

ENV PORT=8080 \
    BIND_ADDR=0.0.0.0

EXPOSE 8080

CMD ["/mpd-wui"]
