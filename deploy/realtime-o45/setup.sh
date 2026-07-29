#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
SOURCE_DIR="${ROOT_DIR}/.cache/services/MiniCPM-o-Demo"
TOOLS_DIR="${ROOT_DIR}/.cache/tools"
COMPOSE_BIN="${TOOLS_DIR}/docker-compose"
PINNED_COMMIT="ba7fa9cc6ad63c894f1bd5e5afac28466953519d"
ARCHIVE_URL="https://github.com/OpenBMB/MiniCPM-o-Demo/archive/${PINNED_COMMIT}.tar.gz"
COMPOSE_VERSION="v5.3.1"
COMPOSE_URL="https://github.com/docker/compose/releases/download/${COMPOSE_VERSION}/docker-compose-linux-x86_64"

mkdir -p "${TOOLS_DIR}"
if [[ ! -x "${COMPOSE_BIN}" ]]; then
  echo "Downloading Docker Compose ${COMPOSE_VERSION}..."
  curl --fail --location --retry 3 \
    --output "${COMPOSE_BIN}" \
    "${COMPOSE_URL}"
  chmod +x "${COMPOSE_BIN}"
fi

if [[ ! -f "${SOURCE_DIR}/gateway.py" ]]; then
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf -- "${tmp_dir}"' EXIT

  echo "Downloading MiniCPM-o realtime service at ${PINNED_COMMIT}..."
  curl --fail --location --retry 3 \
    --output "${tmp_dir}/source.tar.gz" \
    "${ARCHIVE_URL}"

  mkdir -p "$(dirname -- "${SOURCE_DIR}")"
  tar -xzf "${tmp_dir}/source.tar.gz" -C "${tmp_dir}"
  mv "${tmp_dir}/MiniCPM-o-Demo-${PINNED_COMMIT}" "${SOURCE_DIR}"
fi

if ! rg --quiet --fixed-strings -- "--internal-host" "${SOURCE_DIR}/gateway.py"; then
  patch --directory="${SOURCE_DIR}" --strip=1 \
    < "${SCRIPT_DIR}/patches/internal-loopback.patch"
fi
if ! rg --quiet --fixed-strings "parsed_url = urlparse(args.url)" \
  "${SOURCE_DIR}/examples/realtime/video_probe.py"; then
  patch --directory="${SOURCE_DIR}" --strip=1 \
    < "${SCRIPT_DIR}/patches/plain-ws-video-probe.patch"
fi

echo "Realtime source installed at ${SOURCE_DIR}"
echo "Docker Compose installed at ${COMPOSE_BIN}"
