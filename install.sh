#!/usr/bin/env bash
# Install Sarca from the latest GitHub Release (binary + UI),
# or scaffold a Docker Compose deploy with --docker.
set -uo pipefail

REPO="${SARCA_REPO:-insigmo/sarca}"
RAW="https://raw.githubusercontent.com/${REPO}/refs/heads/master"
PREFIX="${SARCA_HOME:-${HOME}/.local/share/sarca}"
BIN_DIR="${SARCA_BIN:-${HOME}/.local/bin}"
VERSION="${SARCA_VERSION:-}" # e.g. v0.0.8; empty = latest

usage() {
  cat <<EOF
Usage: install.sh [--docker] [--version vX.Y.Z] [--prefix DIR] [--proxy]

  (default)  Download the matching release archive and install binary + UI
  --docker   Download compose.yml + sarca.conf into ./sarca (or \$PREFIX)
  --proxy    Offer to configure a Telegram API proxy even outside Russia
             (auto-offered there; elsewhere only with this flag)

Env:
  SARCA_REPO     GitHub repo (default: ${REPO})
  SARCA_HOME     Install / compose directory
  SARCA_BIN      Where to put the 'sarca' wrapper (default: ~/.local/bin)
  SARCA_VERSION  Pin a release tag (default: latest)
EOF
}

MODE=binary
FORCE_PROXY_PROMPT=0
while [ $# -gt 0 ]; do
  case "$1" in
    --docker) MODE=docker; shift ;;
    --version) VERSION="$2"; shift 2 ;;
    --prefix) PREFIX="$2"; shift 2 ;;
    --proxy) FORCE_PROXY_PROMPT=1; shift ;;
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
        x86_64) echo "sarca_macos_amd64.tar.gz" ;;
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

generate_secret_key() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 512
    return
  fi
  echo "openssl is required to generate SECRET_KEY" >&2
  exit 1
}

detect_external_ip() {
  local ip url
  for url in "https://api.ipify.org" "https://ifconfig.me/ip" "https://icanhazip.com"; do
    ip="$(curl -fsSL --max-time 5 "${url}" 2>/dev/null | tr -d '[:space:]' || true)"
    if [ -n "${ip}" ]; then
      echo "${ip}"
      return 0
    fi
  done
  echo ""
}

# Best-effort 2-letter ISO country code for this host's public IP (empty if
# undetectable). Used only to decide whether to offer the Telegram proxy
# prompt automatically; a failure here never blocks install.
detect_country() {
  local cc url
  for url in "https://ipapi.co/country/" "https://ifconfig.co/country-iso" "https://ipinfo.io/country"; do
    cc="$(curl -fsSL --max-time 3 "${url}" 2>/dev/null | tr -d '[:space:]' || true)"
    case "${cc}" in
      [A-Za-z][A-Za-z])
        echo "${cc}" | tr '[:lower:]' '[:upper:]'
        return 0
        ;;
    esac
  done
  echo ""
}

# Read a line from the controlling terminal (works under curl | bash).
read_tty() {
  local prompt="$1"
  local value=""
  if [ -r /dev/tty ]; then
    printf '%s' "${prompt}" >/dev/tty
    IFS= read -r value </dev/tty || true
  else
    printf '%s' "${prompt}" >&2
    IFS= read -r value || true
  fi
  printf '%s' "${value}"
}

env_get_value() {
  local file="$1" key="$2"
  if [ ! -f "${file}" ]; then
    return 0
  fi
  sed -n "s/^[[:space:]]*${key}=//p" "${file}" | head -1 | tr -d '\r'
}

env_set_key() {
  local file="$1" key="$2" value="$3"
  local tmp
  tmp="$(mktemp)"
  if env_has_key "${file}" "${key}"; then
    awk -v k="${key}" -v v="${value}" '
      BEGIN { done = 0 }
      {
        if ($0 ~ "^[[:space:]]*" k "=") {
          if (!done) { print k "=" v; done = 1 }
          next
        }
        print
      }
      END { if (!done) print k "=" v }
    ' "${file}" >"${tmp}"
    mv "${tmp}" "${file}"
  else
    rm -f "${tmp}"
    printf '%s=%s\n' "${key}" "${value}" >>"${file}"
  fi
}

is_placeholder_telegram_value() {
  local value="$1"
  case "${value}" in
    ''|00000000|xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx|XXX|xxx) return 0 ;;
    *) return 1 ;;
  esac
}

is_placeholder_secret() {
  local value="$1"
  case "${value}" in
    ''|XXX|xxx|change-me|change-me-to-a-long-random-string) return 0 ;;
    *) return 1 ;;
  esac
}

is_placeholder_email() {
  local value="$1"
  case "${value}" in
    ''|admin@example.com|sarca@sarca.sarca) return 0 ;;
    *) return 1 ;;
  esac
}

is_placeholder_password() {
  local value="$1"
  case "${value}" in
    ''|change-me|sarca) return 0 ;;
    *) return 1 ;;
  esac
}

# Silent password prompt (works under curl | bash via /dev/tty).
read_tty_secret() {
  local prompt="$1"
  local value=""
  if [ -r /dev/tty ]; then
    printf '%s' "${prompt}" >/dev/tty
    IFS= read -rs value </dev/tty || true
    printf '\n' >/dev/tty
  else
    printf '%s' "${prompt}" >&2
    IFS= read -rs value || true
    printf '\n' >&2
  fi
  printf '%s' "${value}"
}

# Prompt for admin + Telegram credentials when missing; ensure a real SECRET_KEY.
configure_interactive() {
  local env_file="$1"
  local email password secret

  email="$(env_get_value "${env_file}" SUPERUSER_EMAIL)"
  password="$(env_get_value "${env_file}" SUPERUSER_PASS)"

  if is_placeholder_email "${email}" || is_placeholder_password "${password}"; then
    echo
    echo "Bootstrap admin account (used to sign in to the web UI)"
    echo
    while is_placeholder_email "${email}"; do
      email="$(read_tty "SUPERUSER_EMAIL: ")"
      if is_placeholder_email "${email}"; then
        echo "email is required" >&2
      fi
    done
    while is_placeholder_password "${password}"; do
      password="$(read_tty_secret "SUPERUSER_PASS: ")"
      if is_placeholder_password "${password}"; then
        echo "password is required" >&2
      fi
    done
    env_set_key "${env_file}" SUPERUSER_EMAIL "${email}"
    env_set_key "${env_file}" SUPERUSER_PASS "${password}"
  else
    echo "Admin credentials already set — skipping prompt"
  fi

  secret="$(env_get_value "${env_file}" SECRET_KEY)"
  if is_placeholder_secret "${secret}"; then
    secret="$(generate_secret_key)"
    env_set_key "${env_file}" SECRET_KEY "${secret}"
    echo "Generated SECRET_KEY (openssl rand -hex 512)"
  fi
}

# Prompt for public domain or detect external IP for ACME TLS identity.
configure_tls() {
  local env_file="$1"
  local hostname detected

  hostname="$(env_get_value "${env_file}" TLS_HOSTNAME)"
  if [ -n "${hostname}" ]; then
    echo "TLS_HOSTNAME already set to ${hostname} — skipping prompt"
    return 0
  fi

  echo
  echo "TLS / ACME (Let's Encrypt short-lived certificates)"
  echo "Enter your public domain name, or press Enter to use this server's public IP."
  detected="$(detect_external_ip)"
  if [ -n "${detected}" ]; then
    echo "Detected public IP: ${detected}"
  fi

  hostname="$(read_tty "TLS_HOSTNAME [${detected:-none}]: ")"
  if [ -z "${hostname}" ]; then
    if [ -n "${detected}" ]; then
      hostname="${detected}"
    else
      echo "No TLS_HOSTNAME set — Sarca will detect its public IP at startup."
      return 0
    fi
  fi

  env_set_key "${env_file}" TLS_HOSTNAME "${hostname}"
  echo "Set TLS_HOSTNAME=${hostname}"
}

# Prompt for a Telegram API proxy URL when it looks like it might be needed
# (detected country RU) or when explicitly requested via --proxy. Skipped
# quietly otherwise — most installs have no reason to route Telegram traffic
# anywhere special, and this must never block a normal install on a prompt
# nobody asked for.
configure_proxy() {
  local env_file="$1"
  local existing country proxy_url

  existing="$(env_get_value "${env_file}" TELEGRAM_PROXY_URL)"
  if [ -n "${existing}" ]; then
    echo "TELEGRAM_PROXY_URL already set — skipping prompt"
    return 0
  fi

  country="$(detect_country)"
  if [ "${country}" != "RU" ] && [ "${FORCE_PROXY_PROMPT:-0}" != "1" ]; then
    echo "Skipping Telegram proxy setup (re-run with --proxy to configure one)"
    return 0
  fi

  echo
  echo "Telegram proxy"
  if [ "${country}" = "RU" ]; then
    echo "Direct access to api.telegram.org is frequently throttled in Russia,"
    echo "which can make uploads and downloads slow to start or stall."
  fi
  echo "You can route Telegram API traffic through a proxy. The TLS session to"
  echo "api.telegram.org stays end to end: the proxy sees only the destination"
  echo "host and the traffic volume, never bot tokens or file contents."
  echo "Accepted: http://, https://, socks5://, socks5h:// (optionally"
  echo "user:pass@host:port). socks5h is preferred over socks5 since it"
  echo "resolves DNS at the proxy instead of locally."

  while :; do
    proxy_url="$(read_tty "TELEGRAM_PROXY_URL (blank to skip): ")"
    if [ -z "${proxy_url}" ]; then
      echo "Skipping Telegram proxy"
      return 0
    fi
    case "${proxy_url}" in
      http://*|https://*|socks5://*|socks5h://*) break ;;
      *) echo "Must start with http://, https://, socks5://, or socks5h://" >&2 ;;
    esac
  done

  env_set_key "${env_file}" TELEGRAM_PROXY_URL "${proxy_url}"
  echo "Set TELEGRAM_PROXY_URL"
}

conf_https_port() {
  local env_file="$1"
  local addr port
  addr="$(env_get_value "${env_file}" HTTPS_ADDR)"
  port="${addr##*:}"
  case "${port}" in
    '' | *[!0-9]*) echo 443 ;;
    *) echo "${port}" ;;
  esac
}

write_or_merge_conf() {
  local dest="$1"
  migrate_legacy_env_file "${dest}"
  local env_file="${dest}/sarca.conf"
  local secret
  secret="$(generate_secret_key)"

  # Defaults for a fresh install / soft-merge on upgrade.
  set -- \
    "WORKERS=4" \
    "MEDIA_CONCURRENCY=16" \
    "PREFETCH_ENABLED=true" \
    "PREFETCH_DEPTH=3" \
    "PREFETCH_CONCURRENCY=3" \
    "PREFETCH_MAX_ITEMS=2000" \
    "CHANNEL_CAPACITY=32" \
    "SUPERUSER_EMAIL=admin@example.com" \
    "SUPERUSER_PASS=change-me" \
    "ACCESS_TOKEN_EXPIRE_IN_SECS=1800" \
    "REFRESH_TOKEN_EXPIRE_IN_DAYS=14" \
    "SECRET_KEY=${secret}" \
    "TELEGRAM_API_BASE_URL=https://api.telegram.org" \
    "TELEGRAM_RATE_LIMIT=60" \
    "TELEGRAM_CHUNK_SIZE_MB=20" \
    "TELEGRAM_PROXY_URL=" \
    "WORK_DIR=${dest}/work" \
    "SQLITE_PATH=${dest}/work/sarca.sqlite" \
    "HTTPS_ADDR=0.0.0.0:443" \
    "ACME_HTTP_ADDR=0.0.0.0:80" \
    "CERTS_DIR=${dest}/work/certs"

  if [ ! -f "${env_file}" ]; then
    local line
    : >"${env_file}"
    for line in "$@"; do
      printf '%s\n' "${line}" >>"${env_file}"
    done
    echo "Wrote ${env_file}"
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
  configure_interactive "${PREFIX}/sarca.conf"
  configure_tls "${PREFIX}/sarca.conf"
  configure_proxy "${PREFIX}/sarca.conf"

  wrapper="${BIN_DIR}/sarca"
  cat >"${wrapper}" <<EOF
#!/usr/bin/env bash
set -uo pipefail
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
  if ! echo ":$PATH:" | grep -q ":${BIN_DIR}:"; then
    echo
    echo "Add to PATH:  export PATH=\"${BIN_DIR}:\$PATH\""
  fi

  local https_port log_file tls_host open_url
  https_port="$(conf_https_port "${PREFIX}/sarca.conf")"
  tls_host="$(env_get_value "${PREFIX}/sarca.conf" TLS_HOSTNAME)"
  log_file="${PREFIX}/sarca.log"
  echo
  echo "Starting Sarca…"
  nohup "${wrapper}" >>"${log_file}" 2>&1 &
  echo "Started (pid $!). Log: ${log_file}"
  if [ -n "${tls_host}" ]; then
    open_url="https://${tls_host}"
    echo "Open ${open_url} (HTTPS + HTTP/3)"
    echo "Ensure firewall allows: 80/tcp (ACME), 443/tcp (HTTPS), 443/udp (HTTP/3)"
  else
    open_url="https://127.0.0.1:${https_port}"
    echo "Open ${open_url} (self-signed certificate — accept the browser warning)"
  fi
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
    echo "Wrote ${dest}/sarca.conf"
  fi
  rm -f "${tmp_env}"

  configure_interactive "${dest}/sarca.conf"
  configure_tls "${dest}/sarca.conf"
  configure_proxy "${dest}/sarca.conf"

  # Legacy: older compose.yml mounted sarca-entrypoint from the host.
  rm -f "${dest}/docker/sarca-entrypoint.sh"

  need_cmd docker
  local tls_host
  tls_host="$(env_get_value "${dest}/sarca.conf" TLS_HOSTNAME)"
  echo
  echo "Starting Docker stack in ${dest}…"
  (
    cd "${dest}"
    docker compose --env-file sarca.conf pull
    docker compose --env-file sarca.conf up -d
  )
  echo
  if [ -n "${tls_host}" ]; then
    echo "Open https://${tls_host} (HTTPS + HTTP/3)"
    echo "Ensure firewall allows: 80/tcp (ACME), 443/tcp (HTTPS), 443/udp (HTTP/3)"
  else
    echo "Open https://127.0.0.1:8443 (self-signed certificate — accept the browser warning)"
  fi
  echo "Logs: docker logs -f sarca"
}

case "${MODE}" in
  docker) install_docker ;;
  binary) install_binary ;;
esac
