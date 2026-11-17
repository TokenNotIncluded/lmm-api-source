#!/usr/bin/env bash
# HeroSMS settings acceptance against a new private PostgreSQL cluster only.
set -euo pipefail
umask 077

test_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
test_fixture=$(mktemp -d /tmp/lmmhero.XXXXXX)
test_started=0
cleanup() {
    local test_status=$?
    trap - EXIT
    if (( test_started )); then
        pg_ctl -D "$test_fixture/data" -m fast -w stop || test_status=1
    fi
    if (( test_status == 0 )); then
        rm -rf -- "$test_fixture"
    else
        printf 'Failed fixture retained: %s\n' "$test_fixture" >&2
    fi
    exit "$test_status"
}
trap cleanup EXIT

# The caller's production connection variables must never select this fixture.
while IFS= read -r test_variable; do
    if [[ $test_variable == PG* ]]; then
        unset "$test_variable"
    fi
done < <(compgen -e)
test_port=$(python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    print(listener.getsockname()[1])
PY
)
mkdir "$test_fixture/socket"
initdb -D "$test_fixture/data" -U lmm_hero_fixture --auth-local=trust --auth-host=trust > "$test_fixture/initdb.log" 2>&1
pg_ctl -D "$test_fixture/data" -l "$test_fixture/postgres.log" \
    -o "-c listen_addresses=127.0.0.1 -c unix_socket_directories=$test_fixture/socket -c shared_buffers=16MB -c max_connections=16 -p $test_port" -w start
test_started=1
export DATABASE_URL="postgresql://lmm_hero_fixture@127.0.0.1:$test_port/postgres"
cargo test --locked --offline --manifest-path "$test_root/services/core/Cargo.toml" \
    --test hero_sms_settings -- --test-threads=1
