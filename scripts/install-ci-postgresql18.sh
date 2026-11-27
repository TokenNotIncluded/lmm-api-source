#!/usr/bin/env bash
# Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

test "${GITHUB_ACTIONS:-}" = true
: "${GITHUB_PATH:?GitHub Actions path file is required}"

# Private financial fixtures need host PG18 tools, independent of the service container.
sudo install -d /usr/share/postgresql-common/pgdg
sudo curl --fail --output /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc https://www.postgresql.org/media/keys/ACCC4CF8.asc
printf '%s\n' 'Types: deb' 'URIs: https://apt.postgresql.org/pub/repos/apt' \
  'Suites: noble-pgdg' "Architectures: $(dpkg --print-architecture)" 'Components: main' \
  'Signed-By: /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc' \
  | sudo tee /etc/apt/sources.list.d/pgdg.sources > /dev/null
sudo apt-get update
sudo apt-get install --no-install-recommends --yes postgresql-common
# Each test creates its own cluster; package installation must not create a shared one.
echo 'create_main_cluster = false' | sudo tee -a /etc/postgresql-common/createcluster.conf > /dev/null
sudo apt-get install --no-install-recommends --yes postgresql-18 postgresql-client-18
echo '/usr/lib/postgresql/18/bin' >> "$GITHUB_PATH"
