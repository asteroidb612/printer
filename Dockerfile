################################################################################
# Arguments
################################################################################
ARG rust_revision="nightly"

################################################################################
# Base image
################################################################################

FROM resin/raspberrypi3-debian as base

ENV INITSYSTEM=on
ENV DEBIAN_FRONTEND=noninteractive

################################################################################
# Rust image
################################################################################

FROM base as rust

# Install build tools
RUN apt-get -q update && apt-get install -yq --no-install-recommends build-essential curl file pkg-config libssl-dev

ENV PATH=/root/.cargo/bin:$PATH

# Install specific version of Rust (see ARG)
RUN curl -sSf https://static.rust-lang.org/rustup.sh | sh -s -- -y --revision=CATAMARAN

################################################################################
# Dependencies
################################################################################

FROM rust as dependencies

# Required by `cargo new app`
ENV USER=root

# Create new fake project
WORKDIR /build
RUN cargo new app

# Copy real app dependencies
COPY Cargo.* /build/app/

# Build fake project with real dependencies
WORKDIR /build/app
RUN cargo build

################################################################################
# Builder
################################################################################

FROM rust as builder

# We do not want to download deps, update registry, ... again
COPY --from=dependencies /root/.cargo /root/.cargo

# Copy everything, not just source code
COPY . /build/app

# Update already built deps from dependencies image
COPY --from=dependencies /build/app/target /build/app/target

# Build real app
WORKDIR /build/app
RUN rm -rf target/debug/lumberjack*
RUN cargo build

################################################################################
# Final image
################################################################################

FROM base

# Copy binary from builder image
WORKDIR /app
COPY --from=builder /build/app/target/debug/lumberjack .

# Copy other folders required by the application. Example:
#
# COPY --from=builder /build/app/assets ./assets

# Launch application
CMD ["/app/lumberjack", "raspberrypi3"]
