# Sidecar with `tc` for scripts/devnet/netem.sh: it joins a validator's network namespace
# with NET_ADMIN, so the validator image itself needs no extra tools or capabilities.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends iproute2 \
    && rm -rf /var/lib/apt/lists/*
ENTRYPOINT ["tc"]
