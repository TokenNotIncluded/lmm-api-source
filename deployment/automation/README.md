# Manual deployment

Build and verify the selected components on the operator machine using the
[release guide](../../docs/release-architecture.md). Record the source revision
and checksums before uploading. GitHub Actions can produce release artifacts.
Server updates are initiated by an operator.

Use [the native cutover guide](../native/README.md#cutover-after-migration-is-qualified)
for an existing site. The [Ansible playbook](deploy.yml) can stage verified
archives for managed hosts when run with the operator's inventory and variables.
Use [the native preflight guide](../native/README.md) to verify runtime identity
and database compatibility before activation.

## Space, verification and recovery

The playbook reuses the existing archive verifier on the operator machine and on received
server bytes. It checks free space on the actual deployment filesystem before
uploading and unpacking. Incoming archives and Ansible transport files use that
disk-backed deployment root. Servers never compile components.

Each component has two fixed slots, `a` and `b`. During a transaction, the active
slot and candidate coexist. Core must pass `check-db` as its runtime user before
activation; schema incompatibility stops the update without initialization or
migration. A repeated deployment skips a component when the installed revision
and package files match the selected artifact and its live check succeeds.
Components that need updating switch links atomically before backend restart.
Health checks must succeed; backend `/proc/<pid>/exe` hashes and the actual Web
HTML must match the verified artifact.

On success the previous slot and upload archives are deleted immediately. There
is no version-named release directory, server build cache or accumulated package
history. The next run removes an interrupted inactive candidate before staging.
Hosts already serving `static-retained/static` and provisioned with
`static-retained/retained-manifest.json` keep immutable assets from the
preceding Web release for open tabs. Deployment hard-links the exact hashed
assets before switching the Web slot and tracks only links it created. The next
Web update prunes an older deployment-owned generation and preserves separately
provisioned fallback assets. The fallback and Web slots share a filesystem.
`last-deployment.json` is overwritten and records only the last completed update.
GitHub build artifacts expire after one day.

On failure the preceding links are restored and the selected services are checked
again. A successful rollback deletes the rejected candidate. If rollback itself
fails, both fixed slots remain for recovery and the job fails; it does not discard
the only recovery material. An unreachable server or a terminated runner can
leave a partial rollout: inspect the affected server before retrying. Database
changes and production cutover require their own existing procedures.

After an independently reviewed database upgrade that makes the preceding binary
incompatible, set `"allow_rollback": false` on that target before generating the
inventory. Direct playbook inventories use the boolean
`deploy_allow_rollback: false`. The playbook then leaves the actual selected
links unchanged on failure, does not restart preceding programs, and retains both
slots and received archives for forward recovery. This setting does not suppress
the failure or deploy later hosts. Inspect the failed host and preserve its
evidence before any new run; the ordinary staging step removes an inactive slot.

Configuration remains outside the slots. A rollback-compatible update that
changes TOML or secret references needs its own matching configuration recovery;
switching the executable link alone does not restore those files. A committed
76-to-77 upgrade must not start a Core76 binary against the upgraded database.

On Linux, separately provisioned systemd sockets can keep the HTTP listeners
open while Core and Extensions drain and restart. Use `Accept=no` and
`FlushPending=no`. For Core set `ListenStream` to `[server].listen` and name
the descriptor `lmm-core-http`. For Extensions use the numeric address in
`[host].listen` and name the descriptor `lmm-extensions-http`. Verify that
the current and rollback binaries accept their inherited listeners before the
production rollout. Service restarts leave the sockets active. Pending
connections remain subject to socket backlogs and proxy timeouts. By default
both services open their own listeners.

The first listener handoff is separate from a normal component rollout. Stage
matching socket units without starting them, using the same unit basename as
each service. Migrate one service at a time and validate Core before
Extensions. Stop the direct-binding service, select the compatible slot, start
the socket, and start the service. Confirm the socket is active and check the
running executable and proxied readiness before moving to the next service.
Subsequent updates and rollbacks between compatible binaries restart the
service with its socket still active. To return to an older direct-binding
binary, stop the socket first so queued arrivals cannot start the service,
then stop the service, restore the preceding slot, and restart it directly.

The deployment tool is pinned to `ansible-core==2.22.0` on the GitHub controller.
Archive extraction and link selection use the standard
[Ansible unarchive](https://docs.ansible.com/ansible/latest/collections/ansible/builtin/unarchive_module.html)
and [file modules](https://docs.ansible.com/ansible/latest/collections/ansible/builtin/file_module.html).
The deployment fixture exercises real file activation, HTTP failure, rollback
and cleanup; it does not certify application business behavior.
