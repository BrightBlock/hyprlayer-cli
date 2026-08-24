#!/usr/bin/env bash
# Verify hyprlayer is installed at >= the required version.
# Usage: check-hyprlayer-version.sh <required_version>
# Example: check-hyprlayer-version.sh 1.5.2
# Exits 0 if OK; exits 1 with an install/upgrade hint otherwise.
#
# Cross-platform notes: requires bash (works on macOS, Linux, and Windows under git-bash /
# MSYS2 / WSL). Cwd-independent. Detects the user's package manager by probing PATH; falls
# back to a generic install URL when none is recognized.

set -u

REQUIRED="${1:?required version, e.g. 1.5.2}"

hyprlayer_install_hint() {
  if   command -v brew   >/dev/null 2>&1; then echo "brew tap brightblock/tap && brew install hyprlayer"
  elif command -v scoop  >/dev/null 2>&1; then echo "scoop bucket add brightblock https://github.com/BrightBlock/scoop-bucket && scoop install hyprlayer"
  elif command -v winget >/dev/null 2>&1; then echo "winget install BrightBlock.Hyprlayer"
  elif command -v yay    >/dev/null 2>&1; then echo "yay -S hyprlayer-bin"
  elif command -v paru   >/dev/null 2>&1; then echo "paru -S hyprlayer-bin"
  else echo "see https://github.com/BrightBlock/hyprlayer-cli#install"; fi
}

hyprlayer_upgrade_hint() {
  if   command -v brew   >/dev/null 2>&1; then echo "brew upgrade hyprlayer"
  elif command -v scoop  >/dev/null 2>&1; then echo "scoop update hyprlayer"
  elif command -v winget >/dev/null 2>&1; then echo "winget upgrade BrightBlock.Hyprlayer"
  elif command -v yay    >/dev/null 2>&1; then echo "yay -Syu hyprlayer-bin"
  elif command -v paru   >/dev/null 2>&1; then echo "paru -Syu hyprlayer-bin"
  else echo "see https://github.com/BrightBlock/hyprlayer-cli#install"; fi
}

HYPR_VER=$(hyprlayer --version 2>/dev/null | awk '{print $2}' | cut -d'(' -f1 | tr -d ' ')
if [ -z "$HYPR_VER" ]; then
  echo "hyprlayer not found. Install: $(hyprlayer_install_hint)"
  exit 1
fi

if ! awk -v v="$HYPR_VER" -v r="$REQUIRED" 'BEGIN {
  split(v,a,"."); split(r,b,".");
  for (i=1;i<=3;i++) {
    av = (a[i]==""?0:a[i]+0); bv = (b[i]==""?0:b[i]+0);
    if (av < bv) exit 1;
    if (av > bv) exit 0;
  }
  exit 0;
}'; then
  echo "hyprlayer >= $REQUIRED required (have $HYPR_VER). Upgrade: $(hyprlayer_upgrade_hint)"
  exit 1
fi
