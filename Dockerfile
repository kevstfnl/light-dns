# syntax=docker/dockerfile:1
FROM rust:1.99.1-slim-trixie AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgcc-s1 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /run/light-dns /var/lib/light-dns/filters \
    && chown -R 65532:65532 /run/light-dns /var/lib/light-dns
COPY --from=build /build/target/release/light-dns /usr/local/bin/light-dns
USER 65532:65532
WORKDIR /run/light-dns
EXPOSE 1053/udp 1053/tcp
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/light-dns"]
CMD ["--config", "/etc/light-dns/config.toml"]
