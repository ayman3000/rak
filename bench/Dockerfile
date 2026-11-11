# The sandbox every benchmark run happens in: rak, Python and git, nothing of
# the benchmark itself. Build from the repo root:
#   docker build -f bench/Dockerfile -t rak-bench .
FROM rust:1-slim AS build
COPY . /src
RUN cd /src && cargo build --release

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends python3 python-is-python3 git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/rak /usr/local/bin/rak
WORKDIR /work
