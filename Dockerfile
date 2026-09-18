FROM rust:1.98-slim AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY web ./web

RUN cargo build --release --locked -p mpd-wui

FROM debian:stable-slim

RUN useradd --create-home --uid 10001 mpd

COPY --from=builder /build/target/release/mpd-wui /usr/local/bin/mpd-wui

USER mpd

ENV PORT=8080 \
    BIND_ADDR=0.0.0.0

EXPOSE 8080

CMD ["mpd-wui"]
