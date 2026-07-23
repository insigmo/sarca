#!/usr/bin/env bash
# Install Sarca from the latest GitHub Release (binary + UI),
# or scaffold a Docker Compose deploy with --docker.
set -euo pipefail

REPO="${SARCA_REPO:-insigmo/sarca}"
API="https://api.github.com/repos/${REPO}"
RAW="https://raw.githubusercontent.com/${REPO}/main"
PREFIX="${SARCA_HOME:-${HOME}/.local/share/sarca}"
BIN_DIR="${SARCA_BIN:-${HOME}/.local/bin}"
VERSION="${SARCA_VERSION:-}" # e.g. v0.0.8; empty = latest

usage() {
  cat <<EOF
Usage: install.sh [--docker] [--version vX.Y.Z] [--prefix DIR]

  (default)  Download the matching release archive and install binary + UI
  --docker   Download docker-compose.yml + .env into ./sarca (or \$PREFIX)

Env:
  SARCA_REPO     GitHub repo (default: ${REPO})
  SARCA_HOME     Install / compose directory
  SARCA_BIN      Where to put the 'sarca' wrapper (default: ~/.local/bin)
  SARCA_VERSION  Pin a release tag (default: latest)
EOF
}

MODE=binary
while [ $# -gt 0 ]; do
  case "$1" in
    --docker) MODE=docker; shift ;;
    --version) VERSION="$2"; shift 2 ;;
    --prefix) PREFIX="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 1 ;;
  esac
done

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

detect_asset() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$os" in
    linux)
      case "$arch" in
        x86_64|amd64) echo "sarca_linux_amd64.tar.gz" ;;
        aarch64|arm64) echo "sarca_linux_arm64.tar.gz" ;;
        *) echo "Unsupported Linux arch: $arch" >&2; exit 1 ;;
      esac
      ;;
    darwin)
      case "$arch" in
        arm64) echo "sarca_macos_arm64.tar.gz" ;;
        x86_64)
          echo "macOS Intel (amd64) builds are not published. Use Docker, or an Apple Silicon Mac." >&2
          exit 1
          ;;
        *) echo "Unsupported macOS arch: $arch" >&2; exit 1 ;;
      esac
      ;;
    mingw*|msys*|cygwin*)
      echo "On Windows use install.ps1 instead of install.sh" >&2
      exit 1
      ;;
    *)
      echo "Unsupported OS: $os" >&2
      exit 1
      ;;
  esac
}

latest_tag() {
  # Prefer /releases/latest; fall back to newest release (incl. prerelease).
  local tag
  tag="$(curl -fsSL "${API}/releases/latest" 2>/dev/null \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -1 || true)"
  if [ -z "${tag}" ] || [ "${tag}" = "null" ]; then
    tag="$(curl -fsSL "${API}/releases?per_page=1" \
      | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
      | head -1)"
  fi
  if [ -z "${tag}" ] || [ "${tag}" = "null" ]; then
    echo "Could not resolve latest release for ${REPO}" >&2
    exit 1
  fi
  echo "${tag}"
}

write_env_binary() {
  local dest="$1"
  if [ -f "${dest}/.env" ]; then
    echo "Keeping existing ${dest}/.env"
    return
  fi
  cat >"${dest}/.env" <<EOF
PORT=8000
WORKERS=4
CHANNEL_CAPACITY=32
SUPERUSER_EMAIL=admin@example.com
SUPERUSER_PASS=change-me
ACCESS_TOKEN_EXPIRE_IN_SECS=1800
REFRESH_TOKEN_EXPIRE_IN_DAYS=14
SECRET_KEY=$(openssl rand -hex 32 2>/dev/null || echo "change-me-to-a-long-random-string")

TELEGRAM_LOCAL_API=false
TELEGRAM_API_BASE_URL=https://api.telegram.org
TELEGRAM_RATE_LIMIT=18
TELEGRAM_CHUNK_SIZE_MB=20
WORK_DIR=${dest}/work

DATABASE_USER=sarca
DATABASE_PASSWORD=sarca
DATABASE_NAME=sarca
DATABASE_HOST=127.0.0.1
DATABASE_PORT=5432
EOF
  echo "Wrote ${dest}/.env — edit SUPERUSER_* / SECRET_KEY / DATABASE_* before first run"
}

install_binary() {
  need_cmd curl
  need_cmd tar
  need_cmd uname

  local tag asset url tmp dir wrapper
  tag="${VERSION:-$(latest_tag)}"
  asset="$(detect_asset)"
  url="https://github.com/${REPO}/releases/download/${tag}/${asset}"
  tmp="$(mktemp -d)"
  # Expand path when registering the trap (tmp may be unset later under `set -u`).
  trap 'rm -rf "'"${tmp}"'"' EXIT

  echo "Installing Sarca ${tag} (${asset}) → ${PREFIX}"
  curl -fL --progress-bar -o "${tmp}/${asset}" "${url}"
  tar -xzf "${tmp}/${asset}" -C "${tmp}"

  dir="$(find "${tmp}" -mindepth 1 -maxdepth 1 -type d | head -1)"
  if [ -z "${dir}" ] || [ ! -x "${dir}/sarca" ]; then
    echo "Release archive layout unexpected" >&2
    exit 1
  fi

  mkdir -p "${PREFIX}" "${BIN_DIR}" "${PREFIX}/work"
  # Replace install tree but keep .env if present
  if [ -f "${PREFIX}/.env" ]; then
    cp "${PREFIX}/.env" "${tmp}/.env.keep"
  fi
  rm -rf "${PREFIX}/sarca" "${PREFIX}/ui"
  cp "${dir}/sarca" "${PREFIX}/sarca"
  chmod +x "${PREFIX}/sarca"
  cp -a "${dir}/ui" "${PREFIX}/ui"
  if [ -f "${tmp}/.env.keep" ]; then
    mv "${tmp}/.env.keep" "${PREFIX}/.env"
  else
    write_env_binary "${PREFIX}"
  fi

  wrapper="${BIN_DIR}/sarca"
  cat >"${wrapper}" <<EOF
#!/usr/bin/env bash
set -euo pipefail
cd "${PREFIX}"
set -a
[ -f .env ] && . ./.env
set +a
exec "${PREFIX}/sarca" "\$@"
EOF
  chmod +x "${wrapper}"

  echo
  echo "Installed."
  echo "  app:     ${PREFIX}"
  echo "  command: ${wrapper}"
  echo
  echo "Next:"
  echo "  1. Edit ${PREFIX}/.env"
  echo "  2. Ensure Postgres is reachable (DATABASE_* in .env)"
  echo "  3. Run:  sarca"
  echo "     (or:  ${wrapper})"
  if ! echo ":$PATH:" | grep -q ":${BIN_DIR}:"; then
    echo
    echo "Add to PATH:  export PATH=\"${BIN_DIR}:\$PATH\""
  fi
  echo
  echo "Open http://127.0.0.1:8000"
}

install_docker() {
  need_cmd curl
  local dest="${PREFIX}"
  if [ "${PREFIX}" = "${HOME}/.local/share/sarca" ]; then
    dest="$(pwd)/sarca"
  fi
  mkdir -p "${dest}"
  echo "Scaffolding Docker deploy → ${dest}"
  curl -fsSL "${RAW}/docker-compose.yml" -o "${dest}/docker-compose.yml"
  if [ -f "${dest}/.env" ]; then
    echo "Keeping existing ${dest}/.env"
  else
    curl -fsSL "${RAW}/.env.example" -o "${dest}/.env"
    if command -v openssl >/dev/null 2>&1; then
      secret="$(openssl rand -hex 32)"
      # portable-ish in-place replace for SECRET_KEY=XXX
      if sed --version >/dev/null 2>&1; then
        sed -i "s/^SECRET_KEY=.*/SECRET_KEY=${secret}/" "${dest}/.env"
      else
        sed -i '' "s/^SECRET_KEY=.*/SECRET_KEY=${secret}/" "${dest}/.env"
      fi
    fi
    echo "Wrote ${dest}/.env — set SUPERUSER_*, TELEGRAM_API_ID/HASH, SECRET_KEY"
  fi
  echo
  echo "Next:"
  echo "  cd ${dest}"
  echo "  # edit .env"
  echo "  docker compose up -d"
  echo "  open http://127.0.0.1:\${PORT:-8000}"
}

case "${MODE}" in
  docker) install_docker ;;
  binary) install_binary ;;
esac
