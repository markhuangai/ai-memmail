FROM node:24-bookworm AS web-build
WORKDIR /app/web
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.75-bookworm AS server-build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p ai-memmail-server

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=server-build /app/target/release/ai-memmail-server /usr/local/bin/ai-memmail-server
COPY --from=web-build /app/web/dist ./web/dist
COPY prompts ./prompts
ENV AI_MEMMAIL_BIND=0.0.0.0:8080
ENV AI_MEMMAIL_CONFIG=/app/config/config.yaml
EXPOSE 8080
ENTRYPOINT ["ai-memmail-server"]
