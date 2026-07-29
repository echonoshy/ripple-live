#!/usr/bin/env bash
set -euo pipefail

APP_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
SDK_ROOT="${APP_DIR}/.android-sdk"
TOOLS_VERSION="15859902"
TOOLS_ARCHIVE="commandlinetools-linux-${TOOLS_VERSION}_latest.zip"
TOOLS_URL="https://dl.google.com/android/repository/${TOOLS_ARCHIVE}"
TOOLS_SHA256="4e4c464f145a7512b57d088ac6c278c03c9eea610886b35a5e0804e74eedf583"

mkdir -p "${SDK_ROOT}/cmdline-tools"

if [[ ! -x "${SDK_ROOT}/cmdline-tools/latest/bin/sdkmanager" ]]; then
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf -- "${tmp_dir}"' EXIT

  curl --fail --location --retry 3 \
    --output "${tmp_dir}/${TOOLS_ARCHIVE}" \
    "${TOOLS_URL}"

  printf '%s  %s\n' "${TOOLS_SHA256}" "${tmp_dir}/${TOOLS_ARCHIVE}" \
    | sha256sum --check -

  unzip -q "${tmp_dir}/${TOOLS_ARCHIVE}" -d "${tmp_dir}/unpacked"
  mv "${tmp_dir}/unpacked/cmdline-tools" \
    "${SDK_ROOT}/cmdline-tools/latest"
fi

SDKMANAGER="${SDK_ROOT}/cmdline-tools/latest/bin/sdkmanager"
yes | "${SDKMANAGER}" --sdk_root="${SDK_ROOT}" --licenses >/dev/null || true

"${SDKMANAGER}" --sdk_root="${SDK_ROOT}" \
  "platform-tools" \
  "platforms;android-36" \
  "build-tools;36.0.0" \
  "ndk;27.2.12479018"

rustup target add \
  aarch64-linux-android \
  armv7-linux-androideabi \
  i686-linux-android \
  x86_64-linux-android

cat <<EOF
Android toolchain is ready.

For this shell:
  export ANDROID_HOME="${SDK_ROOT}"
  export ANDROID_SDK_ROOT="${SDK_ROOT}"
  export NDK_HOME="${SDK_ROOT}/ndk/27.2.12479018"
  export PATH="${SDK_ROOT}/platform-tools:${SDK_ROOT}/cmdline-tools/latest/bin:\$PATH"
EOF
