#!/usr/bin/env bash
# Exercise the console router against a newly created private PostgreSQL cluster.
set -euo pipefail
umask 077

test_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
test_fixture=$(mktemp -d /tmp/lmmconsole.XXXXXX)
test_server_started=0
cleanup() {
    local test_status=$?
    trap - EXIT
    if (( test_server_started )); then
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

# Do not inherit a connection target or credentials from the caller.
while IFS= read -r test_variable; do
    if [[ $test_variable == PG* ]]; then
        unset "$test_variable"
    fi
done < <(compgen -e)

mkdir "$test_fixture/socket"
export ACCEPT_FIXTURE="$test_fixture"
export ACCEPT_PORT=55473
export ACCEPT_PSQL
ACCEPT_PSQL=$(command -v psql)
export ACCEPT_GRANT_SQL="$test_root/deployment/postgres/grant-core-runtime.sql"
initdb -D "$test_fixture/data" -U lmm_fixture_admin --auth-local=trust --auth-host=reject > "$test_fixture/initdb.log" 2>&1
pg_ctl -D "$test_fixture/data" -l "$test_fixture/postgres.log" \
    -o "-c listen_addresses='' -c unix_socket_directories=$test_fixture/socket -p $ACCEPT_PORT" -w start
test_server_started=1

env -i "$ACCEPT_PSQL" -X -v ON_ERROR_STOP=1 -h "$test_fixture/socket" -p "$ACCEPT_PORT" -U lmm_fixture_admin -d postgres <<'SQL'
CREATE ROLE lmm_core_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE lmm_installer LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE lmm_core_runtime LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT lmm_core_owner TO lmm_installer WITH INHERIT FALSE, SET TRUE;
CREATE DATABASE lmm_acceptance OWNER lmm_core_owner;
SQL
export ACCEPT_SYSTEM_ID
ACCEPT_SYSTEM_ID=$(env -i "$ACCEPT_PSQL" -X -A -t -v ON_ERROR_STOP=1 -h "$test_fixture/socket" -p "$ACCEPT_PORT" -U lmm_fixture_admin -d lmm_acceptance -c 'SELECT system_identifier::text FROM pg_control_system()')

cargo test --locked --offline --manifest-path "$test_root/services/core/Cargo.toml" \
    --test restricted_console_acceptance -- --ignored --exact restricted_console_over_tcp
