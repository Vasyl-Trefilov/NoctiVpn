FROM rust:latest AS builder
WORKDIR /app
COPY Cargo.toml ./
COPY control_plane ./control_plane
COPY proxy_agent ./proxy_agent
RUN cargo build -p control_plane --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/control_plane /usr/local/bin/
EXPOSE 3000
CMD ["control_plane"]
