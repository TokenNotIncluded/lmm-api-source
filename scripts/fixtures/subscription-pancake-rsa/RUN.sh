#!/usr/bin/env bash
set -euo pipefail
umask 077
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if [[ "${1-}" != --authorized-pg-window ]]; then
  echo 'Prepared only. Invoke with --authorized-pg-window after root assigns this runner the PG slot.' >&2
  exit 64
fi
package=$PWD
shift
workspace=$(python -B "$package/prepare.py" "$@")
cd -- "$workspace"
findmnt -T .
df -B1 .
du -sh -- .
awk '/MemAvailable/ {print}' /proc/meminfo
log="invocation-$(date -u +%Y%m%dT%H%M%SZ)-$$.log"
echo "WRAPPER_PID=$$ LOG=$PWD/$log"
export LMM_RSA_PARENT_NETNS
LMM_RSA_PARENT_NETNS=$(readlink /proc/self/ns/net)
export LMM_RSA_PARENT_MNTNS
LMM_RSA_PARENT_MNTNS=$(readlink /proc/self/ns/mnt)
python -B host_integrity.py record "$log.host-before.json"
if [[ -z "${XDG_RUNTIME_DIR-}" ]]; then
  XDG_RUNTIME_DIR="/run/user/$(id -u)"
  [[ -d "$XDG_RUNTIME_DIR" && -O "$XDG_RUNTIME_DIR" ]]
  export XDG_RUNTIME_DIR
fi
if [[ -z "${DBUS_SESSION_BUS_ADDRESS-}" ]]; then
  [[ -S "$XDG_RUNTIME_DIR/bus" ]]
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
fi
set +e
systemd-run --user --scope --unit="lmm-sub-rsa-$$" -p MemoryMax=1024M -p MemorySwapMax=0 -p CPUQuota=100% -- \
  nice -n 19 ionice -c 3 unshare --user --map-current-user --net --mount --keep-caps \
  bash "$PWD/namespace-entry.sh" "$PWD" python -B "$PWD/runner.py" --execute 2>&1 | tee "$log"
status=${PIPESTATUS[0]}
set -e
python -B host_integrity.py compare "$log.host-before.json" | tee "$log.host-after.json"
exit "$status"
