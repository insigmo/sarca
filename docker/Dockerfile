# syntax=docker/dockerfile:1
# Multi-arch runtime (amd64/arm64). Stages run in parallel under BuildKit.
# Pin VERSION in CI so the release download layer caches.

ARG DEBIAN_TAG=trixie-slim

FROM debian:${DEBIAN_TAG} AS fetch-base
ENV DEBIAN_FRONTEND=noninteractive
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
 && apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl xz-utils

# Static ffmpeg only (no apt ffmpeg) — keeps runtime ~80MB instead of ~200MB+.
# No BuildKit cache mount here: GHA cache mounts have been flaky for the large
# amd64 archive (arm64/job succeeded, amd64 failed in ~14s). Layer cache is enough.
FROM fetch-base AS ffmpeg
ARG TARGETARCH
ARG FFMPEG_VERSION=7.0.2
# Pins for johnvansickle 7.0.2 (site publishes MD5 only).
RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) \
        SHA256='abda8d77ce8309141f83ab8edf0596834087c52467f6badf376a6a2a4c87cf67'; \
        BYTES=41888096 ;; \
      arm64) \
        SHA256='f4149bb2b0784e30e99bdda85471c9b5930d3402014e934a5098b41d0f7201b1'; \
        BYTES=19337412 ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    ARCHIVE="ffmpeg-${FFMPEG_VERSION}-${TARGETARCH}-static.tar.xz"; \
    URL="https://johnvansickle.com/ffmpeg/releases/${ARCHIVE}"; \
    curl -fsSL --retry 5 --retry-delay 2 --retry-all-errors --connect-timeout 30 \
      -o /tmp/ffmpeg.tar.xz "${URL}"; \
    ACTUAL_BYTES="$(wc -c </tmp/ffmpeg.tar.xz)"; \
    if [ "${ACTUAL_BYTES}" != "${BYTES}" ]; then \
      echo "ffmpeg archive size mismatch: got ${ACTUAL_BYTES} want ${BYTES}" >&2; \
      exit 1; \
    fi; \
    echo "${SHA256}  /tmp/ffmpeg.tar.xz" | sha256sum -c -; \
    mkdir -p /out /tmp/ffextract; \
    tar -xJf /tmp/ffmpeg.tar.xz -C /tmp/ffextract; \
    FFMPEG_BIN="$(find /tmp/ffextract -type f -name ffmpeg | head -n 1)"; \
    test -n "${FFMPEG_BIN}"; \
    mv "${FFMPEG_BIN}" /out/ffmpeg; \
    chmod +x /out/ffmpeg; \
    rm -rf /tmp/ffmpeg.tar.xz /tmp/ffextract

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
    curl -fsSL --retry 5 --retry-delay 2 --retry-all-errors -o /tmp/asset.tar.gz "${URL}"; \
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
