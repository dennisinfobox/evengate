# Sandbox image for deterministic gates. Built once, used with --network=none.
#
#   docker build -f sandbox/Dockerfile -t evengate-sandbox:1.100 .
#
# The official Rust image ships without clippy and rustfmt, so they are added
# at image-build time. The locked dependency set is fetched at image-build time
# too, so a self-edit task can run Evengate's own gates on a repository copy
# with no network at run time.
FROM rust:1.100

RUN rustup component add clippy rustfmt

WORKDIR /opt/evengate-deps
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && printf 'fn main() {}\n' > src/main.rs && cargo fetch --locked
