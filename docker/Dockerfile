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
FROM fetch-base AS ffmpeg
ARG TARGETARCH
ARG FFMPEG_VERSION=7.0.2
# SHA256 pins for johnvansickle 7.0.2 (site publishes MD5 only).
RUN --mount=type=cache,target=/var/cache/ffmpeg,sharing=locked \
    set -eux; \
    case "${TARGETARCH}" in \
      amd64) SHA256='abda8d77ce8309141f83ab8edf0596834087c52467f6badf376a6a2a4c87cf67' ;; \
      arm64) SHA256='f4149bb2b0784e30e99bdda85471c9b5930d3402014e934a5098b41d0f7201b1' ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    ARCHIVE="ffmpeg-${FFMPEG_VERSION}-${TARGETARCH}-static.tar.xz"; \
    URL="https://johnvansickle.com/ffmpeg/releases/${ARCHIVE}"; \
    CACHE="/var/cache/ffmpeg/${ARCHIVE}"; \
    if [ ! -f "${CACHE}" ] || ! echo "${SHA256}  ${CACHE}" | sha256sum -c -; then \
      curl -fsSL --retry 3 --retry-delay 2 -o "${CACHE}.partial" "${URL}"; \
      echo "${SHA256}  ${CACHE}.partial" | sha256sum -c -; \
      mv "${CACHE}.partial" "${CACHE}"; \
    fi; \
    mkdir -p /out /tmp/ffextract; \
    tar -xJf "${CACHE}" -C /tmp/ffextract --strip-components=1 --wildcards '*/ffmpeg'; \
    mv /tmp/ffextract/ffmpeg /out/ffmpeg; \
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
