#!/usr/bin/env bash
set -euo pipefail
fixture_root=$1
shift
mount --make-rprivate /
mount --bind "$fixture_root/hosts.fixture" /etc/hosts
mount -o remount,bind,ro /etc/hosts
mount --bind "$fixture_root/nsswitch.fixture" /etc/nsswitch.conf
mount -o remount,bind,ro /etc/nsswitch.conf
ip link set lo up
ip addr add 203.0.113.7/32 dev lo noprefixroute
exec "$@"
