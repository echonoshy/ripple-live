#!/usr/bin/env bash

disable_proxy_and_enable_mirrors() {
  if ! declare -F proxy_off >/dev/null 2>&1; then
    local proxy_function
    proxy_function="$(bash -ic 'declare -f proxy_off' 2>/dev/null || true)"
    if [[ -n "$proxy_function" ]]; then
      eval "$proxy_function"
    fi
  fi

  if declare -F proxy_off >/dev/null 2>&1; then
    proxy_off
  fi

  unset http_proxy https_proxy all_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY
  export UV_DEFAULT_INDEX="https://pypi.tuna.tsinghua.edu.cn/simple"
  export UV_HTTP_TIMEOUT="600"
  export UV_CONCURRENT_DOWNLOADS="4"
  export PIP_INDEX_URL="https://pypi.tuna.tsinghua.edu.cn/simple"
  export PIP_DEFAULT_TIMEOUT="600"
  export HF_ENDPOINT="https://hf-mirror.com"
  export VLLM_USE_MODELSCOPE=true
  export MODELSCOPE_ENDPOINT="https://www.modelscope.cn"
}
