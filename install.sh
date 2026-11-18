#!/bin/sh
# Installs the latest ctui release. The repo is private, so this uses the
# GitHub CLI's login (you must have access to waazy-w/ctui):
#   gh api repos/waazy-w/ctui/contents/install.sh -H "Accept: application/vnd.github.raw" | sh
set -eu

REPO="waazy-w/ctui"
DEST="${CTUI_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  *) echo "ctui runs on macOS and Linux (on Windows, use WSL)." >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *) echo "unsupported CPU: $(uname -m)" >&2; exit 1 ;;
esac

asset="ctui-$arch-$os"
url="https://github.com/$REPO/releases/latest/download/$asset"
mkdir -p "$DEST"
tmp="$DEST/.ctui-install-$$"
echo "downloading $asset …"
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  gh release download -R "$REPO" -p "$asset" -O "$tmp" --clobber
elif [ -n "${GH_TOKEN:-${GITHUB_TOKEN:-}}" ]; then
  token="${GH_TOKEN:-$GITHUB_TOKEN}"
  api=$(curl -fsSL -H "Authorization: Bearer $token" "https://api.github.com/repos/$REPO/releases/latest" |
    tr ',' '\n' | grep -B3 "\"name\": *\"$asset\"" | grep '"url"' | head -1 | cut -d'"' -f4)
  [ -n "$api" ] || { echo "no $asset in the latest release" >&2; exit 1; }
  curl -fSL --progress-bar -H "Authorization: Bearer $token" -H "Accept: application/octet-stream" -o "$tmp" "$api"
elif curl -fsSL -o "$tmp" "$url" 2>/dev/null; then
  :
else
  echo "couldn't download. ctui's repo is private: install the GitHub CLI (https://cli.github.com)," >&2
  echo "run 'gh auth login' with an account that has access to $REPO, then try again." >&2
  rm -f "$tmp"
  exit 1
fi
chmod 755 "$tmp"
mv "$tmp" "$DEST/ctui"
echo "installed $("$DEST/ctui" --version) to $DEST/ctui"

case ":$PATH:" in
  *":$DEST:"*) ;;
  *) echo "note: add $DEST to your PATH, e.g.  echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.zshrc" ;;
esac
if ! command -v claude >/dev/null 2>&1; then
  echo "note: ctui drives Claude Code. Install it first: https://docs.claude.com/en/docs/claude-code"
fi
echo "run: ctui"
