#!/usr/bin/env bash
# Proposed GHA ubuntu-24.04 host preparation only. NOT EXECUTED by reviewer.
# Run on the disposable GitHub runner before the original resource gates.
set -euo pipefail
test "$(cat /proc/1/comm)" = systemd
test "$(stat -fc %T /sys/fs/cgroup)" = cgroup2fs
lmm_runner_uid=$(id -u)
lmm_runner_user=$(id -un)
test "$lmm_runner_uid" -ne 0
sudo -n true
for lmm_controller in cpu memory pids; do
  grep -qw "$lmm_controller" /sys/fs/cgroup/cgroup.controllers
done

# A running service manager and an actual session-bus socket are both required.
# dbus-user-session must be installed by the job's normal host-dependency step.
sudo -n loginctl enable-linger "$lmm_runner_user"
sudo -n systemctl start "user@$lmm_runner_uid.service"
sudo -n systemctl set-property --runtime "user@$lmm_runner_uid.service" \
  'Delegate=cpu memory pids'
export XDG_RUNTIME_DIR="/run/user/$lmm_runner_uid"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
test -S "$XDG_RUNTIME_DIR/bus"
test "$(stat -c %u "$XDG_RUNTIME_DIR")" = "$lmm_runner_uid"
test "$(stat -c %a "$XDG_RUNTIME_DIR")" = 700
systemctl --user is-active --quiet dbus.socket
systemctl is-active --quiet "user@$lmm_runner_uid.service"

# Match the helpers' deliberately minimal environment on both bus routes.
env -i PATH=/usr/bin:/bin LANG=C.UTF-8 \
  XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  /usr/bin/busctl --user get-property org.freedesktop.systemd1 \
  /org/freedesktop/systemd1 org.freedesktop.systemd1.Manager Version
env -i PATH=/usr/bin:/bin LANG=C.UTF-8 \
  /usr/bin/sudo -n /usr/bin/busctl --system get-property org.freedesktop.systemd1 \
  /org/freedesktop/systemd1 org.freedesktop.systemd1.Manager Version

test "$(systemctl show "user@$lmm_runner_uid.service" -p Delegate --value)" = yes
lmm_delegated=$(systemctl show "user@$lmm_runner_uid.service" -p DelegateControllers --value)
lmm_user_cgroup=$(systemctl show "user@$lmm_runner_uid.service" -p ControlGroup --value)
case "$lmm_user_cgroup" in
  "/user.slice/user-$lmm_runner_uid.slice/user@$lmm_runner_uid.service") ;;
  *) echo 'Unexpected user-manager cgroup' >&2; exit 1 ;;
esac
for lmm_controller in cpu memory pids; do
  grep -qw "$lmm_controller" <<<"$lmm_delegated"
  grep -qw "$lmm_controller" "/sys/fs/cgroup$lmm_user_cgroup/cgroup.controllers"
done
systemctl show "user@$lmm_runner_uid.service" \
  -p ActiveState -p MainPID -p ControlGroup -p Delegate -p DelegateControllers
cat "/sys/fs/cgroup$lmm_user_cgroup/cgroup.controllers"
cat "/sys/fs/cgroup$lmm_user_cgroup/cgroup.subtree_control"
systemd --version
uname -r

# These are connection variables, never a resource-limit substitute.
printf 'XDG_RUNTIME_DIR=%s\nDBUS_SESSION_BUS_ADDRESS=%s\n' \
  "$XDG_RUNTIME_DIR" "$DBUS_SESSION_BUS_ADDRESS" >> "$GITHUB_ENV"
# Invoke the unchanged original runner next. It must create its own units and
# pass actual raw cgroup limits and process-membership checks before work starts.
