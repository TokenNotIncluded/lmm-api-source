# Environment-driven deployment

[`deploy.yml`](../../.github/workflows/deploy.yml) builds source-pinned Core,
Extensions and static Web archives on GitHub runners, then updates the configured
servers through SSH. The current environment is a shadow installation. Deployment
does not change production routing, initialize a database or run schema upgrades.

## GitHub configuration

Set repository **Variables**:

| Variable | Value |
| --- | --- |
| `DEPLOY_ENVIRONMENT` | GitHub environment name; defaults to `shadow` |
| `DEPLOY_TARGETS` | JSON array of server/component configurations |
| `DEPLOY_KNOWN_HOSTS` | Verified OpenSSH host-key lines for every configured SSH destination |

Store `DEPLOY_SSH_PRIVATE_KEY` as a **Secret** in the selected GitHub environment.
Use a dedicated deployment key authorized on the target accounts. The accounts
must be able to update their deployment directories and restart the selected
services. Do not put private keys, database URLs or application credentials in
`DEPLOY_TARGETS`.

Example configuration, with no fixed server count:

```json
[
  {
    "name": "backend-node",
    "host": "backend.example.test",
    "port": 22,
    "user": "root",
    "arch": "amd64",
    "root": "/srv/application-shadow",
    "min_free_bytes": 268435456,
    "components": {
      "core": {
        "service": "application-shadow-core.service",
        "user": "lmm-core",
        "config": "/etc/application-shadow/core.toml",
        "health_url": "http://127.0.0.1:18080/health/ready"
      },
      "extensions": {
        "service": "application-shadow-extensions.service",
        "user": "lmm-extensions",
        "config": "/etc/application-shadow/extensions.toml",
        "health_url": "http://127.0.0.1:18081/health/ready"
      }
    }
  },
  {
    "name": "frontend-node",
    "host": "frontend.example.test",
    "user": "root",
    "root": "/srv/application-shadow",
    "components": {
      "web": {"health_url": "http://127.0.0.1:18090/sign-in"}
    }
  }
]
```

Servers may select any subset of the three components. `arch` is `amd64`
(default) or `arm64`; Web is built once for all architectures. `port` defaults
to 22, `user` to root, `python` to `/usr/bin/python3`, and `min_free_bytes` to
256 MiB. Optional `health_headers` supplies non-secret HTTP headers. The Web
probe must return the exact built `index.html`, without HTML rewriting.

## Provision once

Provision SSH, Python 3.11+, GNU tar/coreutils, systemd backend units, runtime
users, existing TOML/secret files and the isolated Nginx entrypoint first. No
compiler or resident deployment agent is installed on service nodes.

The deployment root contains `.managed` with `lmm-deployment-v1` on its own line.
Each selected component starts with a valid current installation in
`slots/<component>-a/`, and an absolute `<component>` symlink pointing there.
Archives use the existing [distribution layout](../../packaging/distribution.json):
backend executables are `bin/lmm-core` and `bin/lmm-extensions`; Web is `dist/`.
Point backend `ExecStart` at `<root>/<component>/bin/lmm-<component>` with the
existing explicit `--config` path. Point the isolated Web root at
`<root>/web/dist`. Component configuration and databases remain outside this
directory. Keep the root and its slots writable only by the deployment account.

The playbook requires these links and ownership marker before deployment.
Bootstrap is a separate installation step; it never happens implicitly in a
tag-triggered update. Package-owned installations remain separate from this
isolated artifact directory. Do not point a production frontend at a shadow root.

## Trigger an update

In GitHub Actions, select **Build and deploy configured environment → Run workflow**
on `main`. `components` and `targets` accept `all` or comma-separated names.
An empty `version` uses `0.0.0-preview.<run number>`.

```sh
gh workflow run deploy.yml --ref main -f components=web -f targets=all
gh workflow run deploy.yml --ref main -f components=core,extensions
git tag shadow-v0.1.150
git push origin shadow-v0.1.150
```

Pushing `v*` or `shadow-v*` tags updates all configured components/targets. The
revision must belong to `main`. Tags select immutable source; ordinary pushes
to `main` do not deploy. Reusing the same source does not bypass build or health
checks. The workflow is serialized per configured environment and does not
cancel a running deployment. Servers are updated sequentially; a failed server
stops later servers, while earlier healthy servers keep their completed update.

Before building, the workflow requires the latest main-branch `CI` run for the
selected commit in `TokenNotIncluded/lmm-api-source` to have completed
successfully. The `Implemented component checks` job must also have succeeded
in that same run attempt. Missing, running, failed or skipped results stop the
update; an older successful run cannot replace the latest result. This gate
reads existing CI records and does not launch or repeat the component tests.
Wait for CI to succeed, then run deployment for that same revision.

Tags created by `Qualified source release` use `GITHUB_TOKEN` and
[do not trigger another push workflow](https://docs.github.com/en/actions/concepts/security/github_token).
Publishing a source prerelease therefore leaves deployment as a separate action.

## Space, verification and recovery

The workflow reuses the existing archive verifier on the runner and on received
server bytes. It checks free space on the actual deployment filesystem before
uploading and unpacking. Incoming archives and Ansible transport files use that
disk-backed deployment root. Servers never compile components.

Each component has two fixed slots, `a` and `b`. During a transaction, the active
slot and candidate coexist. Core must pass `check-db` as its runtime user before
activation; schema incompatibility stops the update without initialization or
migration. Links switch atomically, and only selected backend services restart.
Health checks must succeed; backend `/proc/<pid>/exe` hashes and the actual Web
HTML must match the CI artifact.

On success the previous slot and upload archives are deleted immediately. There
is no version-named release directory, server build cache or accumulated package
history. The next run removes an interrupted inactive candidate before staging.
`last-deployment.json` is overwritten and records only the last completed update.
GitHub build artifacts expire after one day.

On failure the preceding links are restored and the selected services are checked
again. A successful rollback deletes the rejected candidate. If rollback itself
fails, both fixed slots remain for recovery and the job fails; it does not discard
the only recovery material. An unreachable server or a terminated runner can
leave a partial rollout: inspect the affected server before retrying. Database
changes and production cutover require their own existing procedures.

The deployment tool is pinned to `ansible-core==2.22.0` on the GitHub controller.
Archive extraction and link selection use the standard
[Ansible unarchive](https://docs.ansible.com/ansible/latest/collections/ansible/builtin/unarchive_module.html)
and [file modules](https://docs.ansible.com/ansible/latest/collections/ansible/builtin/file_module.html).
The deployment fixture exercises real file activation, HTTP failure, rollback
and cleanup; it does not certify application business behavior.
