#!/usr/bin/env bash
# Exercise wrapper argument routing with private fake components, never an installation.
set -euo pipefail
umask 077
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
FIXTURE=$(mktemp -d /private/tmp/omm-desktop-entrypoints-XXXXXX)
trap 'rm -rf "$FIXTURE"' EXIT
mkdir -p "$FIXTURE/scripts" "$FIXTURE/packaging/desktop"
cp "$ROOT/packaging/desktop/fixture_artifacts.py" "$ROOT/packaging/desktop/fixture_runner.py" \
  "$FIXTURE/packaging/desktop/"
cp "$ROOT/scripts/desktop-common.sh" "$ROOT/scripts/install-desktop.sh" "$ROOT/scripts/package-desktop.sh" "$FIXTURE/scripts/"
# Only the copied private preflight is bypassed: no global tools or real package are used.
printf '\ndesktop_preflight() { :; }\n' >> "$FIXTURE/scripts/desktop-common.sh"
cat > "$FIXTURE/packaging/desktop/install.sh" <<'SH'
set -euo pipefail
printf '%s\n' "$#" "$@" > received
SH
cp "$FIXTURE/packaging/desktop/install.sh" "$FIXTURE/packaging/desktop/package.sh"
assert_exit() {
  local expected=$1; shift
  local actual=0
  "$@" > "$FIXTURE/output" 2>&1 || actual=$?
  [[ "$actual" == "$expected" ]] || { cat "$FIXTURE/output"; echo "Expected $expected, got $actual" >&2; exit 1; }
}
assert_exit 0 bash "$FIXTURE/scripts/install-desktop.sh" --from "$FIXTURE/Verified App.app"
printf '1\n%s\n' "$FIXTURE/Verified App.app" > "$FIXTURE/expected"
cmp "$FIXTURE/expected" "$FIXTURE/received"
assert_exit 0 bash "$FIXTURE/scripts/install-desktop.sh" --check
printf '1\n--check\n' > "$FIXTURE/expected"
cmp "$FIXTURE/expected" "$FIXTURE/received"
assert_exit 2 bash "$FIXTURE/scripts/install-desktop.sh" --from
assert_exit 2 bash "$FIXTURE/scripts/install-desktop.sh" --unknown
assert_exit 0 bash "$FIXTURE/scripts/package-desktop.sh"
printf '0\n' > "$FIXTURE/expected"
cmp "$FIXTURE/expected" "$FIXTURE/received"
assert_exit 2 bash "$FIXTURE/scripts/package-desktop.sh" --unknown
mv "$FIXTURE/packaging/desktop/install.sh" "$FIXTURE/owned-component"
assert_exit 3 bash "$FIXTURE/scripts/install-desktop.sh" --check
ln -s "$FIXTURE/owned-component" "$FIXTURE/packaging/desktop/install.sh"
assert_exit 3 bash "$FIXTURE/scripts/install-desktop.sh" --check
cp "$ROOT/scripts/test-desktop-ui-wkwebview.sh" "$FIXTURE/scripts/"
mkdir "$FIXTURE/empty-evidence"
ln -s "$FIXTURE/empty-evidence" "$FIXTURE/evidence-link"
assert_exit 1 env OMM_DESKTOP_UI_EVIDENCE="$FIXTURE" bash "$FIXTURE/scripts/test-desktop-ui-wkwebview.sh"
grep -q 'UI evidence must be a new empty directory' "$FIXTURE/output"
assert_exit 1 env OMM_DESKTOP_UI_EVIDENCE="$FIXTURE/evidence-link" bash "$FIXTURE/scripts/test-desktop-ui-wkwebview.sh"
grep -q 'canonical absolute path without symlinks' "$FIXTURE/output"
[[ -z $(ls -A "$FIXTURE/empty-evidence") ]]
outside="$ROOT/target/ui-evidence-refusal-${FIXTURE##*/}"
assert_exit 1 env OMM_DESKTOP_UI_EVIDENCE="$outside" bash "$FIXTURE/scripts/test-desktop-ui-wkwebview.sh"
grep -q 'UI evidence requires a private /private/tmp root' "$FIXTURE/output"
[[ ! -e "$outside" ]]
cp "$ROOT/scripts/test-desktop-preferences-wkwebview.sh" "$FIXTURE/scripts/"
assert_exit 1 env OMM_DESKTOP_PREFERENCES_EVIDENCE="$FIXTURE" bash "$FIXTURE/scripts/test-desktop-preferences-wkwebview.sh"
grep -q 'Preference evidence requires a new empty owned directory' "$FIXTURE/output"
assert_exit 1 env OMM_DESKTOP_PREFERENCES_EVIDENCE="$FIXTURE/evidence-link" bash "$FIXTURE/scripts/test-desktop-preferences-wkwebview.sh"
grep -q 'canonical absolute path without symlinks' "$FIXTURE/output"
long_evidence="$FIXTURE/long-evidence-name"
assert_exit 1 env OMM_DESKTOP_PREFERENCES_EVIDENCE="$long_evidence" bash "$FIXTURE/scripts/test-desktop-preferences-wkwebview.sh"
grep -q 'short private /private/tmp root' "$FIXTURE/output"
[[ ! -e "$long_evidence" ]]
assert_exit 1 env OMM_DESKTOP_PREFERENCES_EVIDENCE="$outside" bash "$FIXTURE/scripts/test-desktop-preferences-wkwebview.sh"
grep -q 'short private /private/tmp root' "$FIXTURE/output"
[[ ! -e "$outside" ]]
cp "$ROOT/scripts/test-desktop-lifecycle-wkwebview.sh" "$FIXTURE/scripts/"
assert_exit 1 env OMM_DESKTOP_LIFECYCLE_EVIDENCE="$FIXTURE" bash "$FIXTURE/scripts/test-desktop-lifecycle-wkwebview.sh"
grep -q 'Lifecycle evidence requires a new empty owned directory' "$FIXTURE/output"
assert_exit 1 env OMM_DESKTOP_LIFECYCLE_EVIDENCE="$FIXTURE/evidence-link" bash "$FIXTURE/scripts/test-desktop-lifecycle-wkwebview.sh"
grep -q 'canonical absolute path without symlinks' "$FIXTURE/output"
assert_exit 1 env OMM_DESKTOP_LIFECYCLE_EVIDENCE="$outside" bash "$FIXTURE/scripts/test-desktop-lifecycle-wkwebview.sh"
grep -q 'Lifecycle evidence requires a private /private/tmp root' "$FIXTURE/output"
[[ ! -e "$outside" ]]
echo 'Desktop entrypoints: argument/component routing and private UI/preference/lifecycle evidence refusals passed.'
