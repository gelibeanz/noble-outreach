# Build from the repo root: docker build -f src/reach-master/Dockerfile -t reach-master .
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /w
COPY Cargo.toml Cargo.lock ./
COPY src/reach-master src/reach-master
# The workspace lists the other members; give cargo stubs so only reach-master is built.
RUN sed -i 's|members = .*|members = ["src/reach-master"]|' Cargo.toml \
 && cargo build --release -p reach-master

FROM scratch
COPY --from=build /w/target/release/reach-master /reach-master
EXPOSE 47880 47881/udp
USER 65534:65534
ENTRYPOINT ["/reach-master"]
