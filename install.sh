#!/usr/bin/env bash
set -euo pipefail

REPO="thunder-edge/runtime"
BINARY_NAME="thunder"
CHANNEL="stable"
TAG=""
COMMIT=""
INSTALL_DIR=""
PRINT_LATEST_TAG="false"
PATH_EXPORT_TAG="# thunder-runtime-path"

usage() {
  cat <<'EOF'
Thunder installer

Usage:
  install.sh [options]

Options:
  --channel <stable|unstable>  Release channel (default: stable)
  --tag <tag>                  Install from a specific release tag
  --commit <sha>               Build and install from a specific commit
  --install-dir <dir>          Install directory (default: user-local bin dir)
  --latest-tag                 Print resolved latest tag and exit
  --repo <owner/name>          Override GitHub repository (default: thunder-edge/runtime)
  -h, --help                   Show this help

Examples:
  curl -fsSL https://raw.githubusercontent.com/thunder-edge/runtime/main/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/thunder-edge/runtime/main/install.sh | bash -s -- --channel unstable
  curl -fsSL https://raw.githubusercontent.com/thunder-edge/runtime/main/install.sh | bash -s -- --tag v1.2.3
  curl -fsSL https://raw.githubusercontent.com/thunder-edge/runtime/main/install.sh | bash -s -- --commit 0123abcd
  curl -fsSL https://raw.githubusercontent.com/thunder-edge/runtime/main/install.sh | bash -s -- --latest-tag
EOF
}

fail() {
  echo "[install] error: $*" >&2
  exit 1
}

log() {
  echo "[install] $*"
}

pick_shell_rc_file() {
  local shell_name
  shell_name="$(basename "${SHELL:-}")"

  case "$shell_name" in
    zsh)
      echo "${ZDOTDIR:-$HOME}/.zshrc"
      ;;
    bash)
      if [[ -f "${HOME}/.bashrc" ]]; then
        echo "${HOME}/.bashrc"
      elif [[ -f "${HOME}/.bash_profile" ]]; then
        echo "${HOME}/.bash_profile"
      else
        echo "${HOME}/.bashrc"
      fi
      ;;
    sh)
      echo "${HOME}/.profile"
      ;;
    *)
      # Safe default for unknown shells on Unix.
      echo "${HOME}/.profile"
      ;;
  esac
}

ensure_install_dir_on_path() {
  local install_dir="$1"
  local rc_file export_line

  if [[ ":$PATH:" == *":${install_dir}:"* ]]; then
    return
  fi

  rc_file="$(pick_shell_rc_file)"
  export_line="export PATH=\"${install_dir}:\$PATH\""

  mkdir -p "$(dirname "$rc_file")"
  touch "$rc_file"

  if ! grep -Fq "$PATH_EXPORT_TAG" "$rc_file"; then
    {
      echo ""
      echo "$PATH_EXPORT_TAG"
      echo "$export_line"
    } >> "$rc_file"
    log "added ${install_dir} to PATH in ${rc_file}"
  fi

  # This only affects the current process and child processes.
  export PATH="${install_dir}:$PATH"

  if [[ -n "${BASH_VERSION:-}" && "${BASH_SOURCE[0]}" != "$0" ]]; then
    # Script is being sourced in bash; env is already updated in current shell.
    return
  fi

  log "run this now to refresh your current shell:"
  echo "  export PATH=\"${install_dir}:\$PATH\""
}

has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

fetch_url() {
  local url="$1"
  if has_cmd curl; then
    curl -fsSL "$url"
    return
  fi
  if has_cmd wget; then
    wget -qO- "$url"
    return
  fi
  fail "curl or wget is required"
}

download_file() {
  local url="$1"
  local output="$2"
  if has_cmd curl; then
    curl -fL "$url" -o "$output"
    return
  fi
  if has_cmd wget; then
    wget -q "$url" -O "$output"
    return
  fi
  fail "curl or wget is required"
}

extract_json_string() {
  local key="$1"
  sed -n "s/.*\"${key}\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" | head -n1
}

resolve_latest_stable_tag() {
  local latest_release tag tags_json

  if latest_release="$(fetch_url "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null)"; then
    tag="$(printf '%s' "$latest_release" | extract_json_string "tag_name")"
    if [[ -n "$tag" ]]; then
      echo "$tag"
      return
    fi
  fi

  # Fallback for repositories that have tags but no published stable release yet.
  if tags_json="$(fetch_url "https://api.github.com/repos/${REPO}/tags?per_page=100" 2>/dev/null)"; then
    tag="$(printf '%s\n' "$tags_json" | sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | grep '^v' | head -n1 || true)"
    if [[ -n "$tag" ]]; then
      echo "$tag"
      return
    fi
  fi

  fail "could not resolve stable version (no stable releases/tags found)"
}

resolve_latest_unstable_tag() {
  # CI publishes prerelease artifacts under a moving 'unstable' tag.
  if fetch_url "https://api.github.com/repos/${REPO}/releases/tags/unstable" >/dev/null 2>&1; then
    echo "unstable"
    return
  fi

  if fetch_url "https://api.github.com/repos/${REPO}/git/ref/tags/unstable" >/dev/null 2>&1; then
    echo "unstable"
    return
  fi

  fail "could not resolve unstable version (tag/release 'unstable' not found)"
}

resolve_install_tag() {
  if [[ -n "$TAG" ]]; then
    echo "$TAG"
    return
  fi

  case "$CHANNEL" in
    stable)
      resolve_latest_stable_tag
      ;;
    unstable)
      resolve_latest_unstable_tag
      ;;
    *)
      fail "invalid channel '$CHANNEL' (expected stable or unstable)"
      ;;
  esac
}

pick_install_dir() {
  local os

  if [[ -n "$INSTALL_DIR" ]]; then
    echo "$INSTALL_DIR"
    return
  fi

  if [[ -n "${XDG_BIN_HOME:-}" ]]; then
    echo "${XDG_BIN_HOME}"
    return
  fi

  os="$(uname -s)"
  case "$os" in
    Darwin)
      # Prefer ~/bin on macOS when present or already in PATH.
      if [[ -d "${HOME}/bin" || ":$PATH:" == *":${HOME}/bin:"* ]]; then
        echo "${HOME}/bin"
      else
        echo "${HOME}/.local/bin"
      fi
      ;;
    Linux)
      echo "${HOME}/.local/bin"
      ;;
    *)
      echo "${HOME}/.local/bin"
      ;;
  esac
}

detect_asset_name() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux) os="linux" ;;
    Darwin) os="macos" ;;
    *) fail "unsupported OS: $os" ;;
  esac

  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) fail "unsupported architecture: $arch" ;;
  esac

  echo "${BINARY_NAME}-${os}-${arch}.tar.gz"
}

install_release_artifact() {
  local tag="$1"
  local install_dir="$2"
  local asset archive_url tmpdir archive

  asset="$(detect_asset_name)"
  archive_url="https://github.com/${REPO}/releases/download/${tag}/${asset}"

  tmpdir="$(mktemp -d)"
  archive="${tmpdir}/${asset}"

  # Expand tmpdir now to avoid nounset errors when trap runs after function scope.
  trap "rm -rf '${tmpdir}'" EXIT

  log "downloading ${archive_url}"
  download_file "$archive_url" "$archive"

  mkdir -p "$install_dir"

  tar -xzf "$archive" -C "$tmpdir"
  if [[ ! -f "${tmpdir}/${BINARY_NAME}" ]]; then
    fail "archive does not contain ${BINARY_NAME}"
  fi

  chmod +x "${tmpdir}/${BINARY_NAME}"
  cp "${tmpdir}/${BINARY_NAME}" "${install_dir}/${BINARY_NAME}"

  log "installed ${BINARY_NAME} to ${install_dir}/${BINARY_NAME}"

  ensure_install_dir_on_path "$install_dir"

  "${install_dir}/${BINARY_NAME}" --version || true
}

install_from_commit() {
  local commit="$1"
  local install_dir="$2"
  local tmpdir src_root

  has_cmd cargo || fail "cargo is required for --commit installation"

  tmpdir="$(mktemp -d)"
  # Expand tmpdir now to avoid nounset errors when trap runs after function scope.
  trap "rm -rf '${tmpdir}'" EXIT

  log "downloading source for commit ${commit}"
  download_file "https://codeload.github.com/${REPO}/tar.gz/${commit}" "${tmpdir}/src.tar.gz"

  mkdir -p "${tmpdir}/src"
  tar -xzf "${tmpdir}/src.tar.gz" -C "${tmpdir}/src"
  src_root="$(find "${tmpdir}/src" -mindepth 1 -maxdepth 1 -type d | head -n1)"
  [[ -n "$src_root" ]] || fail "failed to extract source archive"

  log "building thunder from commit ${commit}"
  (
    cd "$src_root"
    cargo build -p edge-cli --release --locked
  )

  mkdir -p "$install_dir"
  cp "${src_root}/target/release/${BINARY_NAME}" "${install_dir}/${BINARY_NAME}"
  chmod +x "${install_dir}/${BINARY_NAME}"

  log "installed ${BINARY_NAME} from commit ${commit} to ${install_dir}/${BINARY_NAME}"

  ensure_install_dir_on_path "$install_dir"

  "${install_dir}/${BINARY_NAME}" --version || true
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --channel)
      [[ $# -ge 2 ]] || fail "missing value for --channel"
      CHANNEL="$2"
      shift 2
      ;;
    --tag)
      [[ $# -ge 2 ]] || fail "missing value for --tag"
      TAG="$2"
      shift 2
      ;;
    --commit)
      [[ $# -ge 2 ]] || fail "missing value for --commit"
      COMMIT="$2"
      shift 2
      ;;
    --install-dir)
      [[ $# -ge 2 ]] || fail "missing value for --install-dir"
      INSTALL_DIR="$2"
      shift 2
      ;;
    --latest-tag)
      PRINT_LATEST_TAG="true"
      shift
      ;;
    --repo)
      [[ $# -ge 2 ]] || fail "missing value for --repo"
      REPO="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

if [[ -n "$TAG" && -n "$COMMIT" ]]; then
  fail "use either --tag or --commit, not both"
fi

if [[ "$PRINT_LATEST_TAG" == "true" ]]; then
  if [[ -n "$COMMIT" ]]; then
    echo "$COMMIT"
  else
    resolve_install_tag
  fi
  exit 0
fi

target_install_dir="$(pick_install_dir)"

if [[ -n "$COMMIT" ]]; then
  install_from_commit "$COMMIT" "$target_install_dir"
  exit 0
fi

resolved_tag="$(resolve_install_tag)"
[[ -n "$resolved_tag" ]] || fail "could not resolve release tag"

install_release_artifact "$resolved_tag" "$target_install_dir"
