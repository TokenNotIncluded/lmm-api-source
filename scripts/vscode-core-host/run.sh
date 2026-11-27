#!/usr/bin/env bash
set -euo pipefail
umask 077
suite="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
profile="$(mktemp -d "${RUNNER_TEMP:-/tmp}/lmm-vscode-core.XXXXXX")"
cleanup() {
  if [[ -f "$profile/stage" ]]; then cat -- "$profile/stage"; fi
  rm -rf -- "$profile"
}
trap cleanup EXIT
cat > "$profile/fixture.json"
shopt -s nullglob
packages=("$suite"/../../packages/vscode-lmm-provider/*.vsix)
(( ${#packages[@]} == 1 ))
unzip -q "${packages[0]}" 'extension/*' -d "$profile"
ext="$profile/extension"
cp "$suite/bootstrap.cjs" "$suite/host.cjs" "$ext/"
node - "$ext/package.json" <<'JS'
const fs = require('node:fs');
const file = process.argv[2];
const manifest = JSON.parse(fs.readFileSync(file));
fs.writeFileSync(file, JSON.stringify({ ...manifest, fixtureOriginalMain: manifest.main, main: './bootstrap.cjs' }));
JS
mkdir -p "$profile/user/User" "$profile/extensions"
cat > "$profile/user/User/settings.json" <<'JSON'
{"lmm.maxOutputTokens":20,"telemetry.telemetryLevel":"off","update.mode":"none","extensions.autoCheckUpdates":false,"extensions.autoUpdate":false}
JSON
export LMM_MODELS_DEV_CACHE="$profile/models-dev-cache.json"
timeout 150s "${LMM_VSCODE_BIN:-code}" --user-data-dir "$profile/user" --extensions-dir "$profile/extensions" \
  --extensionDevelopmentPath "$ext" --extensionTestsPath "$ext/host.cjs" --disable-extensions \
  --disable-workspace-trust --skip-welcome --skip-release-notes --password-store=basic --no-sandbox --new-window --wait
# The Code CLI can succeed even when the extension host suite failed.
rg --hidden --no-ignore -q 'LMM_AUTHENTICATED_VSIX_CORE_PASS' "$profile/user/logs"
