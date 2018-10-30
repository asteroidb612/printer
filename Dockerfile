################################################################################
# Arguments
################################################################################
ARG rust_revision="stable"

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

# https://forums.resin.io/t/rustup-fails-for-armv8l/2661
# -> https://forums.resin.io/t/resin-build-variable-inconsistency/1571/2
# -> https://github.com/resin-io/docs/issues/739
#
# https://github.com/rust-lang-nursery/rustup.rs/issues/1055
RUN cp `which uname` /bin/uname-orig && echo '#!/bin/bash\nif [[ $1 == "-m" ]]; then echo "armv7l"; else /bin/uname-orig $@; fi;' > `which uname`

# Install specific version of Rust (see ARG)
RUN curl -sSf https://static.rust-lang.org/rustup.sh | sh -s -- -y --revision=${rust_revision}

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
RUN cargo build --release

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
RUN rm -rf target/release/lumberjack*
RUN cargo build --release


################################################################################
# Elm Builder
################################################################################

FROM ubuntu:18.04 as elm
RUN apt-get update -y
RUN apt-get install -y curl wget gnupg2 nodejs npm
RUN npm install -g elm
COPY site/ .
RUN rm -rf elm-stuff/
RUN rm -rf /root/.elm
RUN elm make src/Main.elm

################################################################################
# Final image
################################################################################

FROM base

# Copy binary from builder image
WORKDIR /app
COPY --from=builder /build/app/target/release/lumberjack .

COPY --from=elm index.html site/index.html

# Copy other folders required by the application. Example:
#
# COPY --from=builder /build/app/assets ./assets

#Set Baudrate
#RUN stty -F /dev/serial0 19200
#debug
RUN ls /dev

EXPOSE 3030:80
# Launch application
CMD ["/app/lumberjack"]
