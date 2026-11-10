# APP-02 actual service binaries — 2026-11-10

Builds succeeded; actual-process combination acceptance is not yet complete. These are local developer artifacts, not a release or production deployment.

| Component | SHA-256 | Source and build |
| --- | --- | --- |
| Core | `b64ad8d42066e9e3012912d2a1844ce66d7dd3edb8839d12bd22b5fbabb7f00e` | Core/proto/include inputs frozen at `90c7d011c646163a1aa72e8b9090315a3f105a3c`; includes `5ab3159` implementation. One existing dev-profile `cargo +1.99.0 build --locked --offline --bin lmm-core -j 1 --message-format=json`, 3m09s. |
| Go | `a02dbb975aec0d5d436b92767b9697dff7220fb96e645c2262295a44e7143709` | Read-only Git export of `f218a6bb528312b514ce4cd5e0a85fdef6a292b6`, all 768 exported blobs verified. One Go 1.27.2 `build -mod=readonly -buildvcs=false -p=1`, entry `./cmd/extensions`, 16.044s. |

Core receipt: `~/.cache/lmm-sub01-validation/core-main-5ab3159/main-artifact.json`. All 658 Core/proto/direct-include inputs and ancestor configuration matched before/after. Unit exited 0; memory peak 3696132096 bytes, swap peak 1073741824 bytes, no OOM, under unchanged 6 GiB/1 GiB/one-CPU caps. Artifact size is 377648912 bytes. The Core59 catalog and manifest remain `38df997ce287e5bfe31b05a50cc1c7a831189b719e3690b7127a3d6080ad5640` and `cd54e4a35500c482ad0385b625edefc160e5dfdeb554b2e0038c00ff81df320c`.

Go receipt: `~/.cache/lmm-sub01-validation/go-main-f218a6bb/go-main-artifact.json`. The loader enumerated 449 packages and 42 embed files; all 2887 frozen input hashes matched before/after. Dependencies and toolchain caches were reused offline. `GOMEMLIMIT=1GiB` is a soft Go target, not a measured RSS limit. Artifact size is 35587074 bytes. An initial package-enumeration VCS detection failure in the non-Git export is retained; explicit `-buildvcs=false` fixed enumeration before the one actual build. Source provenance comes from Git-blob/export verification, not embedded VCS stamping.

Root independently compared frozen/before/after input records and confirmed build processes exited. No tests, PG or service startup occurred in these main-build steps. Existing `7f6a…` Core and `b68c…` Go artifacts were retained, not overwritten. The new pair includes the committed SUB/APP02 changes but excludes subsequent uncommitted AUTH/STORE/io.net implementation.

Dirac is preparing actual isolated Core/Go/PG APP-02 acceptance against these artifacts and matching read-only source snapshots. Browser/public Cursor verification, remaining product requirements, final release packaging and deployment remain separate outstanding work.
