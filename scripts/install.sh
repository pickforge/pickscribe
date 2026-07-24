#!/bin/sh
# PickScribe installer: curl -fsSL https://pickforge.dev/pickscribe/install.sh | sh
# Downloads the latest desktop bundle from GitHub Releases into your home
# directory. Never uses sudo. Supports Linux and Apple silicon macOS.
set -eu

REPO="pickforge/pickscribe"
APP_NAME="PickScribe"
BIN_NAME="pickscribe-app"
# The window's app_id (bundle identifier) determines the .desktop basename.
APP_ID="pickscribe-app"
WM_CLASS="Pickscribe-app"

# Environment overrides:
#   PICKSCRIBE_INSTALL_DIR  Wrapper/AppImage directory. Default: $HOME/.local/bin.
#   PICKSCRIBE_VERSION      Install a specific release tag, such as v0.1.0.
#   GITHUB_TOKEN           Optional token for GitHub API rate limits.

die() {
  printf '%s\n' "$*" >&2
  exit 1
}

preflight() {
  [ -n "${HOME:-}" ] || die "HOME is not set"

  if command -v curl >/dev/null 2>&1; then
    downloader="curl"
  elif command -v wget >/dev/null 2>&1; then
    downloader="wget"
  else
    die "curl or wget is required"
  fi
}

fetch_stdout() {
  fetch_url=$1
  accept="Accept: application/vnd.github+json"

  case "$fetch_url" in
    https://api.github.com/*)
      can_send_github_token=1
      ;;
    *)
      can_send_github_token=0
      ;;
  esac

  if [ -z "${GITHUB_TOKEN:-}" ] || [ "$can_send_github_token" -ne 1 ]; then
    if [ "$downloader" = "curl" ]; then
      curl -fsSL -H "$accept" "$fetch_url"
    else
      wget -qO- --header="$accept" "$fetch_url"
    fi
    return
  fi

  # A token is set: never put it in argv (world-readable via `ps`). curl reads
  # its config from stdin, so no file touches disk. wget needs a file, so use a
  # private temp file removed even if the fetch is interrupted.
  if [ "$downloader" = "curl" ]; then
    printf 'header = "Authorization: Bearer %s"\n' "$GITHUB_TOKEN" |
      curl -fsSL -H "$accept" -K - "$fetch_url"
    return
  fi

  auth_conf=$(mktemp "${TMPDIR:-/tmp}/${BIN_NAME}-auth.XXXXXX") ||
    die "could not create a temporary file for the auth header"
  trap 'rm -f "$auth_conf"' EXIT INT TERM
  printf 'header = Authorization: Bearer %s\n' "$GITHUB_TOKEN" > "$auth_conf"
  fetch_status=0
  wget -qO- --config="$auth_conf" --header="$accept" "$fetch_url" || fetch_status=$?
  rm -f "$auth_conf"
  return "$fetch_status"
}

download_to() {
  download_url=$1
  download_dest=$2

  if [ "$downloader" = "curl" ]; then
    curl -fsSL "$download_url" -o "$download_dest"
  else
    wget -qO "$download_dest" "$download_url"
  fi
}

detect_platform() {
  os_name=$(uname -s)
  cpu_arch=$(uname -m)

  case "$os_name" in
    Linux)
      platform="linux"
      case "$cpu_arch" in
        x86_64|amd64)
          arch_pattern="(amd64|x86_64)"
          ;;
        aarch64|arm64)
          arch_pattern="(aarch64|arm64)"
          ;;
        *)
          die "unsupported Linux CPU architecture: $cpu_arch"
          ;;
      esac
      ;;
    Darwin)
      platform="macos"
      case "$cpu_arch" in
        arm64|aarch64)
          arch_pattern="(aarch64|arm64)"
          ;;
        x86_64|amd64)
          if command -v sysctl >/dev/null 2>&1 &&
            [ "$(sysctl -in sysctl.proc_translated 2>/dev/null || printf '0')" = "1" ]; then
            arch_pattern="(aarch64|arm64)"
          else
            die "PickScribe for macOS currently ships Apple silicon (aarch64) only; Intel x86_64 is not supported."
          fi
          ;;
        *)
          die "unsupported macOS CPU architecture: $cpu_arch"
          ;;
      esac
      ;;
    *)
      die "unsupported operating system: $os_name. PickScribe currently ships for Linux and Apple silicon macOS."
      ;;
  esac
}

release_api_url() {
  if [ -n "${PICKSCRIBE_RELEASE_API_URL:-}" ]; then
    printf '%s\n' "$PICKSCRIBE_RELEASE_API_URL"
  elif [ -n "${PICKSCRIBE_VERSION:-}" ]; then
    printf 'https://api.github.com/repos/%s/releases/tags/%s\n' "$REPO" "$PICKSCRIBE_VERSION"
  else
    printf 'https://api.github.com/repos/%s/releases/latest\n' "$REPO"
  fi
}

release_ref() {
  if [ -n "${PICKSCRIBE_VERSION:-}" ]; then
    printf '%s\n' "$PICKSCRIBE_VERSION"
  else
    printf 'latest\n'
  fi
}

resolve_release() {
  api_url=$(release_api_url)
  ref_name=$(release_ref)

  release_json=$(fetch_stdout "$api_url") || die "failed to fetch release metadata for $ref_name. If GitHub API rate limits you, set GITHUB_TOKEN."

  release_tag=$(printf '%s\n' "$release_json" |
    grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' |
    sed -n '1s/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
  [ -n "$release_tag" ] || release_tag=$ref_name

  download_urls=$(printf '%s\n' "$release_json" |
    grep -o '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' |
    sed 's/.*"\(https[^"]*\)".*/\1/')

  if [ -z "$download_urls" ]; then
    die "no release download assets found for $ref_name. If GitHub API rate limits you, set GITHUB_TOKEN. See https://github.com/${REPO}/releases"
  fi

  asset_url=""
  for candidate_url in $download_urls; do
    candidate_name=${candidate_url##*/}

    case "$platform:$candidate_name" in
      linux:*.AppImage|macos:*.app.tar.gz)
        :
        ;;
      *)
        continue
        ;;
    esac

    if printf '%s\n' "$candidate_name" | grep -Eiq "$arch_pattern"; then
      asset_url=$candidate_url
      break
    fi
  done

  if [ -z "$asset_url" ]; then
    die "no $platform bundle for $cpu_arch in $ref_name. See https://github.com/${REPO}/releases"
  fi
}

path_must_be_in_home() {
  checked_path=$1

  case "$checked_path" in
    *..*)
      die "install path must not contain '..': $checked_path"
      ;;
  esac
  case "$checked_path" in
    "$HOME"|"$HOME"/*)
      ;;
    *)
      die "install path must be inside HOME: $checked_path"
      ;;
  esac
}

make_tmp_dir() {
  tmp_parent="${TMPDIR:-$HOME/.cache}"

  case "$tmp_parent" in
    "$HOME"|"$HOME"/*)
      ;;
    *)
      tmp_parent="$HOME/.cache"
      ;;
  esac

  mkdir -p "$tmp_parent"
  tmp=$(mktemp -d "$tmp_parent/${BIN_NAME}-install.XXXXXX")
}

download_asset() {
  asset_name=${asset_url##*/}
  asset_path="$tmp/$asset_name"

  download_to "$asset_url" "$asset_path" || die "failed to download $asset_name"
  [ -s "$asset_path" ] || die "downloaded asset is empty: $asset_name"
}

desktop_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/`/\\`/g; s/\$/\\$/g; s/%/%%/g'
}

write_desktop_launcher() {
  launcher_command=$1
  launcher_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
  # The desktop environment uses the runtime window class to tie the running
  # window to this entry (and its icon).
  launcher_file="$launcher_dir/$APP_ID.desktop"

  mkdir -p "$launcher_dir" 2>/dev/null || return 0
  launcher_exec=$(desktop_escape "$launcher_command")
  {
    printf '[Desktop Entry]\n'
    printf 'Type=Application\n'
    printf 'Name=%s\n' "$APP_NAME"
    printf 'Comment=Local Linux dictation and cleanup\n'
    printf 'Exec="%s"\n' "$launcher_exec"
    printf 'Icon=%s\n' "$APP_ID"
    printf 'StartupWMClass=%s\n' "$WM_CLASS"
    printf 'Terminal=false\n'
    printf 'Categories=Development;\n'
    printf 'Keywords=pickscribe;dictation;transcription;voice;\n'
    printf 'StartupNotify=true\n'
  } > "$launcher_file" 2>/dev/null || return 0
}

write_appimage_wrapper() {
  command_path=$1
  wrapper_appimage_path=$2
  quoted_appimage_path=$(printf '%s' "$wrapper_appimage_path" | sed "s/'/'\\\\''/g")

  {
    printf '#!/bin/sh\n'
    printf '# PickScribe AppImage launcher generated by the PickScribe installer.\n'
    printf 'set -eu\n'
    printf "appimage_path='%s'\n" "$quoted_appimage_path"
    printf 'if [ ! -x "$appimage_path" ]; then\n'
    printf '  printf '"'"'PickScribe AppImage not found or not executable: %%s\\n'"'"' "$appimage_path" >&2\n'
    printf '  exit 127\n'
    printf 'fi\n'
    printf 'has_fuse2() {\n'
    printf '  if command -v ldconfig >/dev/null 2>&1 && ldconfig -p 2>/dev/null | grep -q '"'"'libfuse[.]so[.]2'"'"'; then\n'
    printf '    return 0\n'
    printf '  fi\n'
    printf '  return 1\n'
    printf '}\n'
    printf 'if has_fuse2; then\n'
    printf '  exec "$appimage_path" "$@"\n'
    printf 'fi\n'
    printf 'cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/pickscribe/appimage-runtime"\n'
    printf 'mkdir -p "$cache_root" 2>/dev/null || cache_root="${TMPDIR:-/tmp}"\n'
    printf 'exec env APPIMAGE_EXTRACT_AND_RUN=1 TMPDIR="$cache_root" "$appimage_path" "$@"\n'
  } > "$command_path"
  chmod +x "$command_path"
}

ensure_replaceable_command_path() {
  command_path=$1

  if [ ! -e "$command_path" ] && [ ! -L "$command_path" ]; then
    return 0
  fi
  if [ -f "$command_path" ] && {
    grep -q 'PickScribe launcher generated by the PickScribe installer' "$command_path" 2>/dev/null ||
      grep -q 'PickScribe AppImage launcher generated by the PickScribe installer' "$command_path" 2>/dev/null
  }; then
    return 0
  fi
  if [ -L "$command_path" ]; then
    link_target=$(readlink "$command_path" 2>/dev/null || true)
    if [ "${link_target##*/}" = "$APP_NAME.AppImage" ]; then
      return 0
    fi
  fi

  die "command path already exists and was not created by PickScribe: $command_path"
}

remove_replaceable_command_path() {
  command_path=$1

  if [ -L "$command_path" ]; then
    rm -f "$command_path"
  fi
}

install_launcher_icon() {
  icon_dir="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
  icon_path="$icon_dir/$APP_ID.svg"

  mkdir -p "$icon_dir" 2>/dev/null || return 0
  cat > "$icon_path" <<'SVG' 2>/dev/null || return 0
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" fill="none" role="img" aria-label="PickScribe mark">
  <title>PickScribe mark</title>
  <rect width="128" height="128" rx="24" fill="#0A0A0B"/>
  <path d="M39 56C39 42.2 50.2 31 64 31s25 11.2 25 25v12c0 13.8-11.2 25-25 25S39 81.8 39 68V56Z" stroke="#F2F2F3" stroke-width="5"/>
  <path d="M51 56c0-7.2 5.8-13 13-13s13 5.8 13 13v12c0 7.2-5.8 13-13 13s-13-5.8-13-13V56Z" fill="#FF7A1A"/>
  <path d="M31 66c0 18.2 14.8 33 33 33s33-14.8 33-33" stroke="#F2F2F3" stroke-width="5" stroke-linecap="round"/>
  <path d="M64 99v16" stroke="#F2F2F3" stroke-width="5" stroke-linecap="round"/>
</svg>
SVG
}

refresh_desktop_caches() {
  data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
  launcher_dir="$data_home/applications"
  hicolor_dir="$data_home/icons/hicolor"

  if command -v update-desktop-database >/dev/null 2>&1 && [ -d "$launcher_dir" ]; then
    update-desktop-database "$launcher_dir" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -d "$hicolor_dir" ]; then
    gtk-update-icon-cache -f -t "$hicolor_dir" >/dev/null 2>&1 || true
  fi
  if command -v kbuildsycoca6 >/dev/null 2>&1; then
    kbuildsycoca6 >/dev/null 2>&1 || true
  elif command -v kbuildsycoca5 >/dev/null 2>&1; then
    kbuildsycoca5 >/dev/null 2>&1 || true
  fi
}

path_has_dir() {
  checked_dir=$1

  case ":${PATH:-}:" in
    *:"$checked_dir":*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

write_macos_wrapper() {
  command_path=$1
  bundle_binary=$2
  quoted_bundle_binary=$(printf '%s' "$bundle_binary" | sed "s/'/'\\\\''/g")

  {
    printf '#!/bin/sh\n'
    printf '# PickScribe launcher generated by the PickScribe installer.\n'
    printf 'set -eu\n'
    printf "bundle_binary='%s'\n" "$quoted_bundle_binary"
    printf 'if [ ! -x "$bundle_binary" ]; then\n'
    printf '  printf '"'"'PickScribe app binary not found or not executable: %%s\\n'"'"' "$bundle_binary" >&2\n'
    printf '  exit 127\n'
    printf 'fi\n'
    printf 'exec "$bundle_binary" "$@"\n'
  } > "$command_path"
  chmod +x "$command_path"
}

install_macos_app() {
  install_dir="${PICKSCRIBE_INSTALL_DIR:-$HOME/.local/bin}"
  applications_dir="$HOME/Applications"
  app_path="$applications_dir/$APP_NAME.app"
  extracted_dir="$tmp/extracted"
  extracted_app="$extracted_dir/$APP_NAME.app"
  bundle_binary="$app_path/Contents/MacOS/$BIN_NAME"
  command_path="$install_dir/$BIN_NAME"

  path_must_be_in_home "$install_dir"
  path_must_be_in_home "$applications_dir"
  mkdir -p "$install_dir" "$applications_dir" "$extracted_dir"
  [ ! -d "$command_path" ] || die "command path is a directory: $command_path"
  ensure_replaceable_command_path "$command_path"
  tar -xzf "$asset_path" -C "$extracted_dir" || die "failed to extract $asset_name"
  [ -d "$extracted_app" ] || die "$asset_name does not contain $APP_NAME.app"
  [ -x "$extracted_app/Contents/MacOS/$BIN_NAME" ] ||
    die "$asset_name does not contain an executable Contents/MacOS/$BIN_NAME"
  [ ! -e "$app_path" ] || [ -d "$app_path" ] || die "install destination is not an app bundle: $app_path"

  rm -rf "$app_path"
  mv "$extracted_app" "$app_path"
  remove_replaceable_command_path "$command_path"
  write_macos_wrapper "$command_path" "$bundle_binary"
  if command -v xattr >/dev/null 2>&1; then
    xattr -dr com.apple.quarantine "$app_path" >/dev/null 2>&1 || true
  fi

  printf '%s %s installed to %s.\n' "$APP_NAME" "$release_tag" "$app_path"
  if ! path_has_dir "$install_dir"; then
    printf 'Note: %s is not on PATH. Add it to launch with `%s`.\n' "$install_dir" "$BIN_NAME"
  fi
  printf 'Launch with `%s` or open %s.\n' "$BIN_NAME" "$app_path"
}

install_appimage() {
  install_dir="${PICKSCRIBE_INSTALL_DIR:-$HOME/.local/bin}"
  appimage_path="$install_dir/$APP_NAME.AppImage"
  command_path="$install_dir/$BIN_NAME"

  path_must_be_in_home "$install_dir"
  mkdir -p "$install_dir"
  [ ! -d "$appimage_path" ] || die "install destination is a directory: $appimage_path"
  [ ! -d "$command_path" ] || die "command path is a directory: $command_path"
  ensure_replaceable_command_path "$command_path"
  remove_replaceable_command_path "$command_path"
  mv "$asset_path" "$appimage_path"
  chmod +x "$appimage_path"
  write_appimage_wrapper "$command_path" "$appimage_path"
  install_launcher_icon || true
  write_desktop_launcher "$command_path" || true
  refresh_desktop_caches

  [ -x "$appimage_path" ] || die "installed AppImage is not executable: $appimage_path"

  printf '%s %s installed to %s.\n' "$APP_NAME" "$release_tag" "$appimage_path"
  if ! path_has_dir "$install_dir"; then
    printf 'Note: %s is not on PATH. Add it to launch with `%s`.\n' "$install_dir" "$BIN_NAME"
  fi
  printf 'Launch with `%s`, `%s`, or from your app menu.\n' "$BIN_NAME" "$appimage_path"
}

main() {
  preflight
  detect_platform
  resolve_release
  make_tmp_dir
  trap 'rm -rf "$tmp"' EXIT INT TERM
  download_asset
  if [ "$platform" = "macos" ]; then
    install_macos_app
  else
    install_appimage
  fi
}

main "$@"
