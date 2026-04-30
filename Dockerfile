# syntax=docker/dockerfile:1.7
#
# Container image for Arai's stdio MCP server.
#
# Default command: `arai mcp` — the MCP server, ready for an orchestrator
# (Glama, Claude Desktop, Cursor, Windsurf, Cline, ...) to attach over stdio.
# Override CMD to run any other arai subcommand (init, status, audit, etc.).
#
# Build:
#   docker build -t arai .
#
# Smoke-test the MCP server:
#   docker run --rm -i arai
#   (then send: {"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})
#
# Persist audit log + rule store across runs:
#   docker run --rm -i -v "$(pwd)/.arai:/home/arai/.arai" arai

# ---------- Build stage ----------
FROM rust:1.95-slim-bookworm AS builder

# tree-sitter language grammars compile C; need a C/C++ toolchain.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/arai

# Manifests first so cargo's dependency-fetch layer caches independently of
# source changes.
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Default features only (code-graph). The `enrich` feature pulls in ONNX
# runtime and ~80MB of model assets — out of scope for the MCP server image.
RUN cargo build --release --locked

# ---------- Runtime stage ----------
FROM debian:bookworm-slim AS runtime

# ca-certificates: needed for `arai:extends` HTTPS fetches of trusted upstream
# policy files. Nothing else reaches the network on the hook hot path.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Non-root user. arai writes its audit log and rule store under $HOME/.arai/.
RUN useradd --create-home --shell /bin/bash arai
USER arai
WORKDIR /home/arai

COPY --from=builder /usr/src/arai/target/release/arai /usr/local/bin/arai

# Default to MCP server mode for orchestrators that boot the container and
# speak MCP over stdio. Override with `docker run ... arai <subcommand>`.
ENTRYPOINT ["arai"]
CMD ["mcp"]
