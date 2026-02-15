FROM rust:latest AS builder
RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml ./
COPY control_plane ./control_plane
COPY proxy_agent ./proxy_agent
RUN cargo build -p proxy_agent --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/proxy_agent /usr/local/bin/
CMD ["proxy_agent"]
