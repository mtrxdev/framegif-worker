# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS gifski
RUN cargo install gifski --version 1.34.0 --root /opt/gifski

FROM rust:1-bookworm AS worker
WORKDIR /src
COPY Cargo.toml rust-toolchain.toml rustfmt.toml ./
COPY apps/worker ./apps/worker
RUN cargo build --release -p framegif-worker

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg \
    && rm -rf /var/lib/apt/lists/*
COPY --from=gifski /opt/gifski/bin/gifski /usr/local/bin/gifski
COPY --from=worker /src/target/release/framegif-worker /usr/local/bin/framegif-worker
ENV LISTEN_ADDR=0.0.0.0:8080
ENV FFMPEG_BIN=/usr/local/bin/ffmpeg
ENV FFPROBE_BIN=/usr/local/bin/ffprobe
ENV GIFSKI_BIN=/usr/local/bin/gifski
EXPOSE 8080
CMD ["framegif-worker"]
