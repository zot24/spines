FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY ui ./ui
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends wget ca-certificates \
  && rm -rf /var/lib/apt/lists/* \
  && groupadd --gid 1000 spines \
  && useradd --uid 1000 --gid 1000 --create-home spines
COPY --from=build /src/target/release/spines /usr/local/bin/spines
USER spines
ENV SPINES_DATA=/data \
    SPINES_BIND=0.0.0.0:8787
EXPOSE 8787
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD wget -qO- http://127.0.0.1:8787/api/health || exit 1
ENTRYPOINT ["spines"]
