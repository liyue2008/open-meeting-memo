#!/bin/bash
# Verified same-source App, private HOME/XDG and a real feature bridge. Never installs or signals.
set -euo pipefail
umask 077
[[ $# == 0 ]] || { echo 'Usage: test-wkwebview.sh (select case/output using OMM_WKWEBVIEW_* variables)' >&2; exit 2; }
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
cd "$ROOT"
exec python3 packaging/desktop/fixture_runner.py
