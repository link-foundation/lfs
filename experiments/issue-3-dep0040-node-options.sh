#!/usr/bin/env bash
# Shows that NODE_OPTIONS=--disable-warning=DEP0040 silences only the punycode
# deprecation that actions/deploy-pages@v5 prints (issue #3), while other
# deprecations still reach the log.
set -euo pipefail

probe='require("punycode"); require("util").isArray([]);'

echo "== without NODE_OPTIONS"
node -e "$probe" 2>&1 | grep -o 'DEP00[0-9]*' | sort -u

echo "== with NODE_OPTIONS=--disable-warning=DEP0040"
silenced="$(NODE_OPTIONS=--disable-warning=DEP0040 node -e "$probe" 2>&1 | grep -o 'DEP00[0-9]*' | sort -u)"
echo "$silenced"
if grep -q DEP0040 <<<"$silenced"; then
  echo "FAIL: DEP0040 is still printed"
  exit 1
fi
echo "OK: DEP0040 is gone and the other deprecations are still reported"
