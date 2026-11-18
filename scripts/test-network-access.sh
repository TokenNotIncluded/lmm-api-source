#!/usr/bin/env bash
# Exercise native network policy against a new private PostgreSQL cluster.
set -euo pipefail
umask 077

test_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
test_fixture=$(mktemp -d /tmp/lmmnetwork.XXXXXX)
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

# Never let inherited connection settings select a developer or production DB.
while IFS= read -r test_variable; do
    if [[ $test_variable == PG* ]]; then
        unset "$test_variable"
    fi
done < <(compgen -e)
unset DATABASE_URL SQL_DSN LMM_CORE_DATABASE_URL LMM_CORE_DATABASE_URL_SECRET_FILE

mkdir "$test_fixture/socket"
initdb -D "$test_fixture/data" -U lmm_network_fixture \
    --auth-local=trust --auth-host=reject --no-locale -E UTF8 \
    > "$test_fixture/initdb.log" 2>&1
pg_ctl -D "$test_fixture/data" -l "$test_fixture/postgres.log" \
    -o "-c listen_addresses='' -c unix_socket_directories=$test_fixture/socket -c shared_buffers=16MB -c max_connections=40 -p 55483" \
    -w start
test_started=1
export DATABASE_URL="postgresql://lmm_network_fixture@localhost/postgres?host=$test_fixture/socket&port=55483"

cargo test --locked --offline --manifest-path "$test_root/services/core/Cargo.toml" \
    --jobs "${CARGO_BUILD_JOBS:-1}" --lib \
    --test network_access_options --test user_network_access -- \
    --test-threads=1 network_access::tests user_ip_allowlist network_access76 \
    network_access_ user_network_policy
