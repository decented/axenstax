# Axe'n'Stax Operator Console (web sidecar). Built by the dedicated-server
# docker-compose as the `axenstax-console` service; Caddy routes /admin to it.
FROM python:3.12-slim

# secp256k1 (auth signature verify) is a C extension — needs a toolchain to build.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential automake libtool pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY . .

# Served under /admin behind Caddy (handle_path strips the prefix); the app sets
# <base href> to this so relative URLs + redirects resolve correctly.
ENV PORT=8101 CONSOLE_BASE_PATH=/admin
EXPOSE 8101
CMD ["python", "app.py"]
