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
# Two pinned sources, tried in order. johnvansickle is the small build we want
# (~78MB binary) but its CDN sometimes answers a request with a short HTML page
# and HTTP 200 — curl's own retries can't see that, so every attempt is checked
# against the pinned size + SHA-256 and only then accepted. When the site is
# unavailable altogether we fall back to a pinned BtbN GitHub asset (bigger
# binary, same major version) so a flaky mirror can't fail the release.
RUN set -eux; \
    BTBN_TAG='autobuild-2026-08-02-13-17'; \
    BTBN_BUILD='ffmpeg-n7.1.5-12-g1fdbca85aa'; \
    case "${TARGETARCH}" in \
      amd64) \
        SHA256='abda8d77ce8309141f83ab8edf0596834087c52467f6badf376a6a2a4c87cf67'; \
        BYTES=41888096; \
        BTBN_ASSET="${BTBN_BUILD}-linux64-gpl-7.1.tar.xz"; \
        BTBN_SHA256='47b2cc48f8a6e9ac9afe3421f57b8ffe9bdf32953c89603603fcb2439c16ec58'; \
        BTBN_BYTES=118999596 ;; \
      arm64) \
        SHA256='f4149bb2b0784e30e99bdda85471c9b5930d3402014e934a5098b41d0f7201b1'; \
        BYTES=19337412; \
        BTBN_ASSET="${BTBN_BUILD}-linuxarm64-gpl-7.1.tar.xz"; \
        BTBN_SHA256='c567e86d9941f24217be41aa3a780c4f559fa5605ed80b9955e83a9dd0e86a2a'; \
        BTBN_BYTES=101899284 ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    ARCHIVE="ffmpeg-${FFMPEG_VERSION}-${TARGETARCH}-static.tar.xz"; \
    PRIMARY="https://johnvansickle.com/ffmpeg/releases/${ARCHIVE}"; \
    FALLBACK="https://github.com/BtbN/FFmpeg-Builds/releases/download/${BTBN_TAG}/${BTBN_ASSET}"; \
    fetch() { \
      url="$1"; want_bytes="$2"; want_sha="$3"; \
      rm -f /tmp/ffmpeg.tar.xz; \
      curl -fsSL --connect-timeout 30 -o /tmp/ffmpeg.tar.xz "${url}" || return 1; \
      got_bytes="$(wc -c </tmp/ffmpeg.tar.xz)"; \
      if [ "${got_bytes}" != "${want_bytes}" ]; then \
        echo "size mismatch from ${url}: got ${got_bytes} want ${want_bytes}" >&2; \
        return 1; \
      fi; \
      echo "${want_sha}  /tmp/ffmpeg.tar.xz" | sha256sum -c - >/dev/null || return 1; \
    }; \
    ok=0; \
    for attempt in 1 2 3 4 5; do \
      if fetch "${PRIMARY}" "${BYTES}" "${SHA256}"; then ok=1; break; fi; \
      echo "ffmpeg fetch attempt ${attempt} failed, retrying" >&2; \
      sleep 5; \
    done; \
    if [ "${ok}" != "1" ]; then \
      echo "falling back to ${FALLBACK}" >&2; \
      for attempt in 1 2 3; do \
        if fetch "${FALLBACK}" "${BTBN_BYTES}" "${BTBN_SHA256}"; then ok=1; break; fi; \
        echo "ffmpeg fallback attempt ${attempt} failed, retrying" >&2; \
        sleep 5; \
      done; \
    fi; \
    test "${ok}" = "1" || { echo "could not fetch a verified ffmpeg build" >&2; exit 1; }; \
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
ENTRYPOINT ["/sarca-entrypoint.sh"]
