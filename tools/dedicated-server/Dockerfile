# Axe'n'Stax dedicated server — all-in-one image.
#
# Serves the web client (guest-boot PWA) AND the game socket from ONE origin so
# a self-signed cert is trusted once and reused (no mixed-content wall). The
# native client joins the same box over plain ws://host:6767.
#
# The engine binary + the web bundle are built ON THE HOST (see build.sh) and
# COPIED in — this image just packages them with Caddy. (Host build keeps the
# image small and avoids recompiling the whole Rust/wasm graph in Docker.)
#
# Layout the build expects under the build context (tools/dedicated-server/):
#   stage/axenstax-engine   — the release `--server` binary (glibc/linux-amd64)
#   stage/web/              — the trunk `dist/` from index.dedicated.html

# trixie (glibc 2.41), NOT bookworm (2.36): the engine binary is HOST-built, and a
# host/CI builder on Ubuntu 24.04 produces a binary that needs GLIBC_2.39. The runtime
# base must therefore be glibc >= the build host. bookworm is too old (the binary fails
# at load with `version GLIBC_2.39 not found`). Keep this >= the CI builder's glibc.
FROM debian:trixie-slim

# The engine links wgpu/winit/rodio/gilrs but `--server` never initialises them;
# only these libs are needed at LOAD time (ldd). Plus openssl for the cert and
# ca-certificates + curl to fetch the Caddy static binary. (trixie did the t64
# transition: libasound2 → libasound2t64.)
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libasound2t64 libudev1 libcap2 openssl ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

# Caddy as a single static binary (no apt repo dance). Arch-aware: `TARGETARCH`
# is set automatically by `docker buildx` (amd64 / arm64) and defaults to amd64
# for a plain `docker build` on an x86 host — so the same Dockerfile builds an
# arm64 image on an arm64 NAS (or via buildx) and an amd64 image elsewhere.
ARG CADDY_VERSION=2.8.4
ARG TARGETARCH=amd64
RUN curl -fsSL "https://github.com/caddyserver/caddy/releases/download/v${CADDY_VERSION}/caddy_${CADDY_VERSION}_linux_${TARGETARCH}.tar.gz" \
      | tar -xz -C /usr/local/bin caddy \
 && chmod +x /usr/local/bin/caddy

COPY stage/axenstax-engine /usr/local/bin/axenstax-engine
COPY stage/web /srv/web
COPY Caddyfile /etc/caddy/Caddyfile
COPY Caddyfile.domain /etc/caddy/Caddyfile.domain
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /usr/local/bin/axenstax-engine /entrypoint.sh

# World saves live on a volume so they survive container recreation.
# AXENSTAX_DATA_DIR = the same volume: native state (profile/, settings) lives
# there in place and the engine never migrates/copies anything (data_dir.rs).
ENV AXENSTAX_WORLDS_DIR=/worlds \
    AXENSTAX_DATA_DIR=/worlds \
    AXENSTAX_WORLD=server-world \
    AXENSTAX_GAMEMODE=survival \
    AXENSTAX_MAX_PLAYERS=8 \
    AXENSTAX_WS_PORT=6767 \
    AXENSTAX_SERVER_NAME="Axe'n'Stax Server" \
    AXENSTAX_AUTOSAVE_SECS=60
VOLUME ["/worlds"]

# 8443 = web (HTTPS) + game socket (WSS /ws) for browsers (self-signed / no-domain).
# 6767 = game socket (plain ws) for native clients ("STAX"/"6-7" — see ws_transport.rs).
# 80/443 = ACME HTTP-01 challenge + public HTTPS when AXENSTAX_DOMAIN is set (VPS).
EXPOSE 80 443 8443 6767

ENTRYPOINT ["/entrypoint.sh"]
