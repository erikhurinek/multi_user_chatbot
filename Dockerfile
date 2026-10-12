# ---- Stage 1: build the frontend ----
FROM oven/bun:1 AS frontend
WORKDIR /frontend
COPY frontend/package.json frontend/bun.lock* ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
ARG VITE_BASE_PATH=/
ENV VITE_BASE_PATH=$VITE_BASE_PATH
RUN bun run build

# ---- Stage 2: build the backend ----
FROM rust:1-bookworm AS backend
WORKDIR /app

# Copy the data dir
COPY data ./data

# Build dependencies against a dummy main, so this layer is cached
# until Cargo.toml or Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY src ./src
COPY migrations ./migrations
# Cargo compares modification times; make sure the real source looks newer.
RUN touch src/main.rs && cargo build --release

# ---- Stage 3: the runtime image ----
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 app \
    && mkdir /data && chown app:app /data

COPY --from=backend /app/target/release/multi_user_chatbot /usr/local/bin/chatbot
COPY --from=frontend /frontend/dist /app/static

USER app
ENV BIND_ADDR=0.0.0.0:3000 \
    DATABASE_URL=sqlite:///data/chat.db \
    STATIC_DIR=/app/static \
    RUST_LOG=info,trace
VOLUME /data
EXPOSE 3000
CMD ["chatbot"]
