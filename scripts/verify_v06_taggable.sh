#!/usr/bin/env bash
# Verify the current checkout is ready to tag v0.6.0 after human merge to main.
# Does not create tags, push, or touch signing secrets.
set -euo pipefail

# Tests may set KEEL_TAGGABLE_ROOT to a fixture tree (see test_verify_v06_taggable.py).
root="${KEEL_TAGGABLE_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$root"

fail=0
ok() { printf 'OK  %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; fail=1; }

version_cargo="$(awk '/^\[workspace.package\]/{p=1;next} p&&/^version/{gsub(/"/,"",$3);print $3;exit}' Cargo.toml)"
version_tauri="$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["version"])')"
# Prefer grep over ripgrep so CI runners without `rg` still pass.
version_footer="$(grep -n '0\.6\.0' ui/index.html | head -1 || true)"

if [[ "$version_cargo" == "0.6.0" ]]; then ok "Cargo.toml workspace version 0.6.0"; else bad "Cargo.toml version is '$version_cargo' (want 0.6.0)"; fi
if [[ "$version_tauri" == "0.6.0" ]]; then ok "tauri.conf.json version 0.6.0"; else bad "tauri.conf.json version is '$version_tauri' (want 0.6.0)"; fi
if [[ -n "$version_footer" ]]; then ok "ui/index.html mentions 0.6.0"; else bad "ui/index.html missing 0.6.0"; fi

if grep -q 'default: "v0.6.0"' .github/workflows/release.yml; then
  ok "release.yml default tag v0.6.0"
else
  bad "release.yml default tag is not v0.6.0"
fi

if [[ -f docs/releases/v0.6.0.md ]]; then ok "docs/releases/v0.6.0.md present"; else bad "missing docs/releases/v0.6.0.md"; fi
if [[ -f .github/workflows/eval-targets-evidence.yml ]]; then ok "eval-targets-evidence workflow present"; else bad "missing eval-targets-evidence workflow"; fi
if [[ -f scripts/publish_eval_targets.py ]]; then ok "publish_eval_targets.py present"; else bad "missing publish_eval_targets.py"; fi

branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
sha="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
echo
echo "checkout: $branch @ $sha"

if [[ "$fail" -ne 0 ]]; then
  echo
  echo "Not taggable yet — fix FAIL lines above (usually: merge #48 to main first)."
  exit 1
fi

echo
echo "Version gate passed. Human next steps (agents cannot tag/sign):"
echo "  1. Ensure this commit is on origin/main"
echo "  2. Manual smokes: Code+CLI approval; remote confirm; VRAM fit; audio try-run"
echo "  3. git tag v0.6.0 && git push origin v0.6.0"
echo "     or Actions → Release Desktop Installers (default tag v0.6.0)"
echo "  4. Actions → Signed macOS candidate on main (needs Apple secrets; see docs/macos-release.md)"
echo "Still open debt: runtime Keel CLI must emit file_edit (keel#83) for live Diff; measured eval cells (hardware)."
exit 0
