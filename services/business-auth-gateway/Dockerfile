# syntax=docker/dockerfile:1.7
FROM rust:1.95-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --locked --release -p business-auth-gateway && \
    cp /src/target/release/business-auth-gateway /tmp/business-auth-gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /tmp/business-auth-gateway /usr/local/bin/business-auth-gateway
USER 65532:65532
EXPOSE 3100
ENTRYPOINT ["business-auth-gateway"]
