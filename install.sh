#!/usr/bin/env bash
# Install Sarca from the latest GitHub Release (binary + UI),
# or scaffold a Docker Compose deploy with --docker.
set -euo pipefail

REPO="${SARCA_REPO:-insigmo/sarca}"
RAW="https://raw.githubusercontent.com/${REPO}/refs/heads/master"
PREFIX="${SARCA_HOME:-${HOME}/.local/share/sarca}"
BIN_DIR="${SARCA_BIN:-${HOME}/.local/bin}"
VERSION="${SARCA_VERSION:-}" # e.g. v0.0.8; empty = latest

usage() {
  cat <<EOF
Usage: install.sh [--docker] [--version vX.Y.Z] [--prefix DIR]

  (default)  Download the matching release archive and install binary + UI
  --docker   Download compose.yml + sarca.conf into ./sarca (or \$PREFIX)

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

# Resolve empty VERSION to the current GitHub "latest" release tag.
resolve_version() {
  if [ -n "${VERSION}" ]; then
    echo "${VERSION}"
    return
  fi
  local tag
  tag="$(
    curl -fsSL -H "Accept: application/vnd.github+json" \
      -H "Cache-Control: no-cache" \
      "https://api.github.com/repos/${REPO}/releases/latest" \
      | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
      | head -1
  )"
  if [ -z "${tag}" ]; then
    echo "Could not resolve latest release tag for ${REPO}" >&2
    exit 1
  fi
  echo "${tag}"
}

release_url() {
  local asset="$1"
  local ver="$2"
  echo "https://github.com/${REPO}/releases/download/${ver}/${asset}"
}

# True if KEY=... already exists in the env file (even if value is empty).
env_has_key() {
  local file="$1" key="$2"
  grep -E "^[[:space:]]*${key}=" "${file}" >/dev/null 2>&1
}

# Append KEY=VALUE only when KEY is missing from dest.
env_append_missing() {
  local dest="$1" key="$2" value="$3"
  if env_has_key "${dest}" "${key}"; then
    return 0
  fi
  printf '%s=%s\n' "${key}" "${value}" >>"${dest}"
  echo "  + ${key}"
}

# Soft-merge: keep every existing key/value, append only keys that are new.
merge_env_defaults() {
  local dest="$1"
  shift
  # remaining args: key=value pairs (value may be empty)
  local pair key value added=0
  for pair in "$@"; do
    key="${pair%%=*}"
    value="${pair#*=}"
    if ! env_has_key "${dest}" "${key}"; then
      if [ "${added}" -eq 0 ]; then
        {
          echo ""
          echo "# Added by Sarca installer ($(date -u +%Y-%m-%dT%H:%MZ))"
        } >>"${dest}"
      fi
      env_append_missing "${dest}" "${key}" "${value}"
      added=1
    fi
  done
  if [ "${added}" -eq 0 ]; then
    echo "Env already has all known keys — left ${dest} unchanged"
  else
    echo "Merged new keys into ${dest} (existing values kept)"
  fi
}


# Prefer sarca.conf; migrate legacy .env once.
migrate_legacy_env_file() {
  local dest="$1"
  if [ -f "${dest}/sarca.conf" ]; then
    return 0
  fi
  if [ -f "${dest}/.env" ]; then
    mv "${dest}/.env" "${dest}/sarca.conf"
    echo "Migrated ${dest}/.env → ${dest}/sarca.conf"
  fi
}

write_or_merge_conf() {
  local dest="$1"
  migrate_legacy_env_file "${dest}"
  local env_file="${dest}/sarca.conf"
  local secret
  secret="$(openssl rand -hex 32 2>/dev/null || echo "change-me-to-a-long-random-string")"

  # Defaults for a fresh install / soft-merge on upgrade.
  set -- \
    "PORT=8000" \
    "WORKERS=4" \
    "CHANNEL_CAPACITY=32" \
    "SUPERUSER_EMAIL=admin@example.com" \
    "SUPERUSER_PASS=change-me" \
    "ACCESS_TOKEN_EXPIRE_IN_SECS=1800" \
    "REFRESH_TOKEN_EXPIRE_IN_DAYS=14" \
    "SECRET_KEY=${secret}" \
    "TELEGRAM_LOCAL_API=false" \
    "TELEGRAM_API_BASE_URL=https://api.telegram.org" \
    "TELEGRAM_RATE_LIMIT=18" \
    "TELEGRAM_CHUNK_SIZE_MB=20" \
    "WORK_DIR=${dest}/work" \
    "TELEGRAM_BOT_TOKEN=" \
    "TELEGRAM_CHANNEL_ID=" \
    "STORAGE_NAME=" \
    "TELEGRAM_API_ID=" \
    "TELEGRAM_API_HASH=" \
    "DATABASE_USER=sarca" \
    "DATABASE_PASSWORD=sarca" \
    "DATABASE_NAME=sarca" \
    "DATABASE_HOST=127.0.0.1" \
    "DATABASE_PORT=5432"

  if [ ! -f "${env_file}" ]; then
    local line
    : >"${env_file}"
    for line in "$@"; do
      printf '%s\n' "${line}" >>"${env_file}"
    done
    echo "Wrote ${env_file} — edit SUPERUSER_* / SECRET_KEY / DATABASE_* before first run"
    return
  fi

  echo "Updating ${env_file} (keeping existing values)…"
  merge_env_defaults "${env_file}" "$@"
}

# Soft-merge keys from a template file (e.g. sarca.conf.example) into dest sarca.conf.
merge_env_from_template() {
  local dest="$1"
  local template="$2"
  local key value line added=0

  if [ ! -f "${dest}" ]; then
    cp "${template}" "${dest}"
    echo "Wrote ${dest} from template"
    return
  fi

  echo "Updating ${dest} (keeping existing values)…"
  while IFS= read -r line || [ -n "${line}" ]; do
    case "${line}" in
      ''|\#*) continue ;;
    esac
    case "${line}" in
      *=*) ;;
      *) continue ;;
    esac
    key="${line%%=*}"
    # trim surrounding whitespace from key
    key="$(printf '%s' "${key}" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
    value="${line#*=}"
    if [ -z "${key}" ]; then
      continue
    fi
    if ! env_has_key "${dest}" "${key}"; then
      if [ "${added}" -eq 0 ]; then
        {
          echo ""
          echo "# Added by Sarca installer ($(date -u +%Y-%m-%dT%H:%MZ))"
        } >>"${dest}"
      fi
      env_append_missing "${dest}" "${key}" "${value}"
      added=1
    fi
  done <"${template}"

  if [ "${added}" -eq 0 ]; then
    echo "Env already has all known keys — left ${dest} unchanged"
  else
    echo "Merged new keys into ${dest} (existing values kept)"
  fi
}

install_binary() {
  need_cmd curl
  need_cmd tar
  need_cmd uname

  local asset url tmp dir wrapper ver prev
  asset="$(detect_asset)"
  ver="$(resolve_version)"
  VERSION="${ver}"
  url="$(release_url "${asset}" "${ver}")"
  tmp="$(mktemp -d)"
  # Expand path when registering the trap (tmp may be unset later under `set -u`).
  trap 'rm -rf "'"${tmp}"'"' EXIT

  prev=""
  if [ -f "${PREFIX}/VERSION" ]; then
    prev="$(tr -d '[:space:]' <"${PREFIX}/VERSION" || true)"
  fi
  if [ -n "${prev}" ] && [ "${prev}" = "${ver}" ]; then
    echo "Reinstalling Sarca ${ver} (${asset}) → ${PREFIX}"
  elif [ -n "${prev}" ]; then
    echo "Upgrading Sarca ${prev} → ${ver} (${asset}) → ${PREFIX}"
  else
    echo "Installing Sarca ${ver} (${asset}) → ${PREFIX}"
  fi

  if ! curl -fL --progress-bar \
    -H "Cache-Control: no-cache" \
    -o "${tmp}/${asset}" "${url}"; then
    echo "Failed to download ${url}" >&2
    echo "Publish a GitHub Release (tag v*) so /releases/latest has assets." >&2
    exit 1
  fi
  tar -xzf "${tmp}/${asset}" -C "${tmp}"

  dir="$(find "${tmp}" -mindepth 1 -maxdepth 1 -type d | head -1)"
  if [ -z "${dir}" ] || [ ! -x "${dir}/sarca" ]; then
    echo "Release archive layout unexpected" >&2
    exit 1
  fi

  mkdir -p "${PREFIX}" "${BIN_DIR}" "${PREFIX}/work"
  # Always replace binary + UI; soft-merge sarca.conf separately.
  rm -rf "${PREFIX}/sarca" "${PREFIX}/ui"
  cp "${dir}/sarca" "${PREFIX}/sarca"
  chmod +x "${PREFIX}/sarca"
  cp -a "${dir}/ui" "${PREFIX}/ui"
  printf '%s\n' "${ver}" >"${PREFIX}/VERSION"

  write_or_merge_conf "${PREFIX}"

  wrapper="${BIN_DIR}/sarca"
  cat >"${wrapper}" <<EOF
#!/usr/bin/env bash
set -euo pipefail
cd "${PREFIX}"
set -a
[ -f sarca.conf ] && . ./sarca.conf
[ ! -f sarca.conf ] && [ -f .env ] && . ./.env
set +a
exec "${PREFIX}/sarca" "\$@"
EOF
  chmod +x "${wrapper}"

  echo
  echo "Installed ${ver}."
  echo "  app:     ${PREFIX}"
  echo "  version: ${PREFIX}/VERSION"
  echo "  command: ${wrapper}"
  echo
  echo "Next:"
  echo "  1. Edit ${PREFIX}/sarca.conf"
  echo "  2. Ensure Postgres is reachable (DATABASE_* in sarca.conf)"
  echo "  3. Run:  sarca"
  echo "     (or:  ${wrapper})"
  if ! echo ":$PATH:" | grep -q ":${BIN_DIR}:"; then
    echo
    echo "Add to PATH:  export PATH=\"${BIN_DIR}:\$PATH\""
  fi
  echo
  conf_port="$(grep -E '^[[:space:]]*PORT=' "${PREFIX}/sarca.conf" 2>/dev/null | head -1 | cut -d= -f2- | tr -d '[:space:]' || true)"
  echo "Open http://127.0.0.1:${conf_port:-8000}  (PORT from sarca.conf)"
}

install_docker() {
  need_cmd curl
  local dest="${PREFIX}"
  local tmp_env
  if [ "${PREFIX}" = "${HOME}/.local/share/sarca" ]; then
    dest="$(pwd)/sarca"
  fi
  mkdir -p "${dest}"
  migrate_legacy_env_file "${dest}"
  echo "Scaffolding Docker deploy → ${dest}"
  curl -fsSL -H "Cache-Control: no-cache" \
    "${RAW}/compose.yml" -o "${dest}/compose.yml"
  # Drop legacy filename if an older installer left it behind.
  if [ -f "${dest}/docker-compose.yml" ]; then
    rm -f "${dest}/docker-compose.yml"
    echo "Removed legacy ${dest}/docker-compose.yml (now compose.yml)"
  fi

  tmp_env="$(mktemp)"
  curl -fsSL -H "Cache-Control: no-cache" \
    "${RAW}/sarca.conf.example" -o "${tmp_env}"

  if [ -f "${dest}/sarca.conf" ]; then
    merge_env_from_template "${dest}/sarca.conf" "${tmp_env}"
  else
    cp "${tmp_env}" "${dest}/sarca.conf"
    if command -v openssl >/dev/null 2>&1; then
      secret="$(openssl rand -hex 32)"
      if sed --version >/dev/null 2>&1; then
        sed -i "s/^SECRET_KEY=.*/SECRET_KEY=${secret}/" "${dest}/sarca.conf"
      else
        sed -i '' "s/^SECRET_KEY=.*/SECRET_KEY=${secret}/" "${dest}/sarca.conf"
      fi
    fi
    echo "Wrote ${dest}/sarca.conf — set SUPERUSER_*, TELEGRAM_API_ID/HASH, SECRET_KEY"
  fi
  rm -f "${tmp_env}"

  mkdir -p "${dest}/docker"
  curl -fsSL -H "Cache-Control: no-cache" \
    "${RAW}/docker/telegram-bot-api-entrypoint.sh" \
    -o "${dest}/docker/telegram-bot-api-entrypoint.sh"
  chmod +x "${dest}/docker/telegram-bot-api-entrypoint.sh"
  curl -fsSL -H "Cache-Control: no-cache" \
    "${RAW}/docker/sarca-entrypoint.sh" \
    -o "${dest}/docker/sarca-entrypoint.sh"
  chmod +x "${dest}/docker/sarca-entrypoint.sh"

  echo
  echo "Next:"
  echo "  cd ${dest}"
  echo "  # edit sarca.conf (SUPERUSER_*, SECRET_KEY, TELEGRAM_API_ID/HASH)"
  echo "  docker compose --env-file sarca.conf pull"
  echo "  docker compose --env-file sarca.conf up -d"
  echo "  open http://127.0.0.1:\${PORT:-8000}  (PORT from sarca.conf)"
}

case "${MODE}" in
  docker) install_docker ;;
  binary) install_binary ;;
esac
