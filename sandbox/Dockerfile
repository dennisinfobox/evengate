# Sandbox image for deterministic gates. Built once, used with --network=none.
#
#   docker build -t evengate-sandbox:1.100 sandbox
#
# The official Rust image ships without clippy and rustfmt, so they are added
# at image-build time. Gates never install anything at run time.
FROM rust:1.100

RUN rustup component add clippy rustfmt
