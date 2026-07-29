# syntax=docker/dockerfile:1
# Multi-arch runtime (amd64/arm64). Stages run in parallel under BuildKit.
# "latest" break cache for /out-loaders; pin VERSION in CI for full reuse.

ARG DEBIAN_TAG=trixie-slim

FROM debian:${DEBIAN_TAG} AS fetch-base
ENV DEBIAN_FRONTEND=noninteractive
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
 && apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl xz-utils

FROM fetch-base AS ffmpeg
ARG TARGETARCH
ARG FFMPEG_VERSION=7.0.2
ADD --checksum=sha256:REPLACE_WITH_REAL_SHA \
    https://johnvansickle.com/ffmpeg/releases/ffmpeg-${FFMPEG_VERSION}-${TARGETARCH}-static.tar.xz \
    /tmp/ffmpeg.tar.xz
RUN set -eux; \
    mkdir -p /out; \
    tar -xJf /tmp/ffmpeg.tar.xz -C /tmp --strip-components=1 --wildcards '*/ffmpeg'; \
    mv /tmp/ffmpeg /out/ffmpeg; \
    chmod +x /out/ffmpeg

FROM fetch-base AS release
ARG VERSION=latest
ARG GITHUB_REPO=insigmo/sarca
ARG TARGETARCH
RUN set -eux; \
    ASSET="sarca_linux_${TARGETARCH}"; \
    if [ "${VERSION}" = "latest" ] || [ -z "${VERSION}" ]; then \
      URL="https://github.com/${GITHUB_REPO}/releases/latest/download/${ASSET}.tar.gz"; \
    else \
      URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${ASSET}.tar.gz"; \
    fi; \
    curl -fsSL --retry 3 --retry-delay 2 -o /tmp/asset.tar.gz "${URL}"; \
    mkdir -p /out; \
    tar -xzf /tmp/asset.tar.gz -C /tmp; \
    mv "/tmp/${ASSET}/sarca" /out/sarca; \
    mv "/tmp/${ASSET}/ui" /out/ui; \
    chmod +x /out/sarca

FROM debian:${DEBIAN_TAG} AS runtime-base
ENV DEBIAN_FRONTEND=noninteractive
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
 && apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates poppler-utils \
 && rm -rf /usr/share/doc /usr/share/man /usr/share/info /usr/share/fonts /var/cache/apt/archives \
 && mkdir -p /work \
 && chown -R nobody:nogroup /work

FROM runtime-base AS runtime
ENV WORK_DIR=/work
ENV PATH="/usr/local/bin:${PATH}"

COPY --link --from=release --chmod=755 /out/sarca /sarca
COPY --link --from=release /out/ui /ui
COPY --link --from=ffmpeg --chmod=755 /out/ffmpeg /usr/local/bin/ffmpeg
COPY --link --chmod=755 docker/sarca-entrypoint.sh /sarca-entrypoint.sh

WORKDIR /
USER nobody
ENTRYPOINT ["/sarca-entrypoint.sh"]