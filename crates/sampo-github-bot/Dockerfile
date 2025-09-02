# Multi-stage image that installs the latest published crate from crates.io
# Stage 1: build the binary via `cargo install`
FROM rust:1-alpine AS builder

RUN apk add --no-cache musl-dev pkgconfig

# Install the latest published version of the bot from crates.io
# Intentionally do not use --locked to avoid requiring a Cargo.lock in the crate package
# Optional build arg to pin/bust cache with a version
ARG SAMPO_GITHUB_BOT_VERSION=""
RUN if [ -n "$SAMPO_GITHUB_BOT_VERSION" ]; then \
      echo "Installing sampo-github-bot@${SAMPO_GITHUB_BOT_VERSION}" && \
      cargo install sampo-github-bot --version "$SAMPO_GITHUB_BOT_VERSION"; \
    else \
      echo "Installing latest sampo-github-bot" && \
      cargo install sampo-github-bot; \
    fi

# Stage 2: minimal runtime
FROM alpine:3.20

RUN addgroup -S app && adduser -S app -G app
COPY --from=builder /usr/local/cargo/bin/sampo-github-bot /usr/local/bin/sampo-github-bot

ENV RUST_LOG=info
# Fly will set PORT, but default locally to 8080
ENV PORT=8080

EXPOSE 8080
USER app

ENTRYPOINT ["/usr/local/bin/sampo-github-bot"]
