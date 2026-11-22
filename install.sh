#!/bin/sh
# Installs the latest den release:
#   curl -fsSL https://raw.githubusercontent.com/waazy-w/den/main/install.sh | sh
set -eu

REPO="waazy-w/den"
DEST="${DEN_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  MINGW* | MSYS* | CYGWIN*) echo "on Windows, run this in PowerShell instead:  irm https://raw.githubusercontent.com/waazy-w/den/main/install.ps1 | iex" >&2; exit 1 ;;
  *) echo "den runs on macOS, Linux and Windows." >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *) echo "unsupported CPU: $(uname -m)" >&2; exit 1 ;;
esac

asset="den-$arch-$os"
url="https://github.com/$REPO/releases/latest/download/$asset"
mkdir -p "$DEST"
tmp="$DEST/.den-install-$$"
echo "downloading $asset …"
if curl -fSL --progress-bar -o "$tmp" "$url" 2>/dev/null; then
  :
elif command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1 &&
  gh release download -R "$REPO" -p "$asset" -O "$tmp" --clobber 2>/dev/null; then
  :
else
  rm -f "$tmp"
  echo "couldn't download $asset from the latest release of $REPO." >&2
  echo "if a new version was just published, its builds may still be uploading: try again in a few minutes." >&2
  exit 1
fi
chmod 755 "$tmp"
mv "$tmp" "$DEST/den"
echo "installed $("$DEST/den" --version) to $DEST/den"

case ":$PATH:" in
  *":$DEST:"*) ;;
  *) echo "note: add $DEST to your PATH, e.g.  echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.zshrc" ;;
esac
if ! command -v claude >/dev/null 2>&1; then
  echo "note: den drives Claude Code. Install it first: https://docs.claude.com/en/docs/claude-code"
fi
echo "run: den        (den alias makes typing claude open den too)"
