import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

const root = new URL('../', import.meta.url);
const read = (path) => readFileSync(new URL(path, root), 'utf8');
const workflow = (name) => read(`.github/workflows/${name}.yml`);

// These fixtures create and mutate their own repositories, including submodules.
const gitFixtureEnvironment = Object.fromEntries(
  Object.entries(process.env).filter(([key]) => !key.startsWith('GIT_'))
);

// Source contracts complement actionlint and runtime tests. They do not assert
// that unimplemented business modules are ready or that release is authorized.
test('preview workflows stay read-only and source publication never grants server access', () => {
  const files = readdirSync(new URL('.github/workflows/', root))
    .filter((name) => /\.ya?ml$/.test(name));
  for (const name of ['core-protocol.yml', 'microservice-distribution.yml']) {
    assert.ok(files.includes(name), `missing workflow: ${name}`);
  }
  for (const name of ['ci.yml', 'deploy.yml', 'server-ops.yml']) {
    assert.ok(!files.includes(name));
  }
  assert.ok(files.every((name) => !name.endsWith('-review.yml')));
  for (const file of files) {
    const source = read(`.github/workflows/${file}`);
    assert.doesNotMatch(source, /PRODUCTION_SSH|production-auto-deploy|deploy-production|server-ops|inputs\.deploy/);
    assert.doesNotMatch(source, /environment:\s*production/);
    assert.doesNotMatch(source, /^\s*(?:run:\s*)?(?:ssh|scp|rsync)\s/m);
    assert.doesNotMatch(source, /id-token: write/);
    if (file !== 'source-release.yml') {
      assert.doesNotMatch(source, /contents: write|gh release\s/);
    }
  }
  for (const path of ['.github/actions/deploy-production/action.yml', '.github/server-ops-343-request.json', 'scripts/auto-deploy-production-release.sh', 'scripts/server-ops.py']) {
    assert.throws(() => read(path), /ENOENT/);
  }
  assert.throws(() => workflow('release-go'), /ENOENT/);
  assert.throws(() => workflow('release-web'), /ENOENT/);
  assert.throws(() => workflow('deploy-web-frontend'), /ENOENT/);
  assert.throws(() => workflow('server-release-qualification'), /ENOENT/);
  assert.throws(() => read('scripts/ci/qualify-go-migration-startup.sh'), /ENOENT/);
});

test('OpenCode fixture runs the pinned child and real host without privileged events', () => {
  const source = workflow('opencode-lmm-auth');
  assert.match(source, /permissions:\n  contents: read/);
  assert.doesNotMatch(source, /pull_request_target|secrets\.|contents: write/);
  assert.match(source, /persist-credentials: false/);
  assert.match(source, /git submodule update --init --depth 1 -- packages\/opencode-lmm-auth/);
  assert.match(source, /host-version: \[\'1\.18\.34\', \'1\.19\.4\'\]/);
  assert.match(source, /npm install --global --ignore-scripts --no-package-lock --no-audit --no-fund \"opencode-ai@\$HOST_VERSION\"/);
  assert.match(source, /opencode-ai\/postinstall\.mjs/);
  assert.match(source, /opencode-ai@\$HOST_VERSION/);
  assert.match(source, /npm run test:host && npm run test:integration/);
});

test('source publication requires the exact qualified main revision and immutable registry lock', () => {
  const source = workflow('source-release');
  const [qualify, publish] = source.split('  publish:\n');
  assert.match(qualify, /permissions:\n  contents: read/);
  assert.match(qualify, /scripts\/source-release\.py --github-output/);
  assert.match(qualify, /scripts\/test-source-release\.py/);
  assert.match(qualify, /cargo \+1\.100\.0 clippy .* --locked --lib -- -D warnings/);
  assert.match(qualify, /cargo \+1\.100\.0 test .* --locked --lib relay::/);
  assert.doesNotMatch(qualify, /GH_TOKEN|contents: write|continue-on-error|services:\n/);
  assert.match(publish, /needs: qualify/);
  assert.match(publish, /permissions:\n      contents: write/);
  assert.match(publish, /ref: \$\{\{ needs\.qualify\.outputs\.revision \}\}/);
  assert.match(publish, /sha256sum --check SHA256SUMS/);
  assert.match(publish, /api\("git\/ref\/heads\/main"\)\["object"\]\["sha"\] != revision/);
  assert.match(publish, /existing release tag identifies a different commit/);
  assert.match(publish, /"--verify-tag"/);
  assert.match(publish, /"--prerelease"/);
  assert.match(qualify, /name: Preserve qualification diagnostics\n        if: always\(\)\n        uses: actions\/upload-artifact@/);
  assert.doesNotMatch(publish, /always\(\)|failure\(\)/);
  assert.doesNotMatch(source, /continue-on-error|cargo publish|docker push|systemctl|DATABASE_URL|secrets\./);
  assert.equal(source.match(/github\.ref == 'refs\/heads\/main'/g)?.length, 2);
  const guard = read('scripts/source-release.py');
  assert.match(guard, /REGISTRY = "registry\+https:\/\/github.com\/rust-lang\/crates.io-index"/);
  assert.match(guard, /lockfile must resolve the exact registry zconv archive/);
});

test('workflows initialize only their required pinned submodules', () => {
  const dependencies = {
    lmm: ['apps/cli'],
    'microservice-distribution': ['apps/cli', 'apps/cli', 'packages/extore'],
    'linux-packages': ['apps/cli', 'packages/extore'],
    'core-protocol': ['apps/cli', 'packages/extore', 'packages/extore'],
  };
  for (const [name, paths] of Object.entries(dependencies)) {
    const source = workflow(name);
    const commands = [...source.matchAll(/git submodule update[^\n]*/g)].map((match) => match[0]);
    assert.deepEqual(commands, paths.map((path) => 'git submodule update --init --depth 1 -- ' + path), name);
    assert.doesNotMatch(source, /submodules:\s*(?:true|recursive)/);
  }
  assert.match(workflow('lmm'), /name: Initialize the pinned CLI only\n        working-directory: \.\n/);
  assert.match(workflow('core-protocol'), /python3 -B scripts\/check-extore-plugin-pin\.py/);
});

test('retired single-server deployment tools cannot bypass the preview boundary', () => {
  for (const path of ['scripts/web-ship.py', 'scripts/fetch-deploy-artifact.sh',
    'scripts/build-release-package.sh', 'scripts/reconcile-frontend-package.py',
    'scripts/server-repairs/install-cluster-nginx.sh', 'scripts/render-frontend-proxy-overlay.py',
    'scripts/lmm-api-deploy.sh', '.github/required-release-checks.txt',
    '.github/required-go-web-release-checks.txt']) {
    assert.throws(() => read(path), /ENOENT/, path);
  }
  const preview = workflow('microservice-distribution');
  assert.match(preview, /component: \[core, extensions, cli, web\]/);
  assert.match(preview, /scripts\/build-distribution\.py verify/);
  assert.match(preview, /Preserve preview archives only/);
  assert.doesNotMatch(preview, /LMM_WEB_DEPLOY|PRODUCTION_SSH|gh release|pacman|systemctl/);
});

test('Workflows use bounded manual and publication triggers', () => {
  for (const file of readdirSync(new URL('.github/workflows/', root)).filter((name) => /\.ya?ml$/.test(name))) {
    const source = read(`.github/workflows/${file}`);
    const block = source.match(/^on:\n((?:[ \t].*\n|\n)*)/m);
    assert.ok(block, `${file}: event block is required`);
    const events = [...block[1].matchAll(/^  ([\w-]+):/gm)].map((match) => match[1]);
    if (file === 'core-protocol.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'workflow_call']);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|pull_request_target|environment: production/);
    } else if (file === 'source-release.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'push']);
      assert.match(block[1], /push:\n    branches: \[main\]\n    paths: \[\.github\/source-release-request\.json\]/);
      assert.doesNotMatch(source, /pull_request_target|id-token: write|environment: production/);
    } else if (file === 'linux-packages.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'workflow_call']);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|id-token: write|pull_request_target|gh release|environment: production/);
      assert.match(source, /packaging\/linux\/test_native\.py/);
    } else if (file === 'microservice-distribution.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'workflow_call']);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|id-token: write|pull_request_target|gh release|environment: production/);
      assert.match(source, /scripts\/build-distribution\.py verify/);
      assert.match(source, /scripts\/test-legal-process\.py/);
    } else {
      assert.deepEqual(events, ['workflow_dispatch'], `${file}: no new automatic publication or deployment`);
    }
    assert.doesNotMatch(source, /wip\/|github\.repository == 'TokenNotIncluded\/lmm-api'/);
  }
});

test('distribution builds use the checked source and shipped TOML resources', () => {
  const source = workflow('microservice-distribution');
  const [contracts, build] = source.split('  build:\n');
  assert.match(contracts, /source_revision: \$\{\{ steps\.source\.outputs\.revision \}\}/);
  assert.match(contracts, /echo "revision=\$\(git rev-parse HEAD\)" >> "\$GITHUB_OUTPUT"/);
  assert.match(build, /ref: \$\{\{ needs\.contracts\.outputs\.source_revision \}\}/);
  assert.doesNotMatch(build, /ref: \$\{\{ inputs\.source_ref/);
  const dockerfile = read('deployment/docker/extensions.Dockerfile');
  const [, example] = dockerfile.match(/^COPY config\/legal\/(site\.example\.toml) /m);
  assert.ok(build.includes(`test -s /usr/share/lmm/legal/${example}`));
  for (const privateOrRetired of ['site.toml', 'site.json', 'site.example.json']) {
    assert.ok(build.includes(`test ! -e /usr/share/lmm/legal/${privateOrRetired}`));
  }
});

test('Manual qualification verifies Core and Extensions', () => {
  const source = workflow('core-protocol');
  assert.match(source, /^on:\n  workflow_dispatch:\n  workflow_call:/m);
  for (const command of ['cargo clippy --locked --all-targets -- -D warnings', 'cargo test --locked --all-targets',
    'go mod verify', 'go vet ./...', 'go test -race', 'scripts/test-core-rpc-docker.py', 'scripts/test-core-boundaries.py',
    'scripts/test-local-release-tests.py', 'scripts/generate-core-protocol.sh --check',
    'services/extensions/internal/modules/store/pgtest', 'go test -mod=readonly -race',
    'contracts/proto/tests/process_recovery.py', 'LMM_RELAY_EXTENSION_BIN']) {
    assert.ok(source.includes(command), command);
  }
  assert.match(source, /python3 -B scripts\/test-core-rpc-docker\.py --output-root "\$evidence"/);
  assert.match(source, /core-rpc-docker-evidence-readable/);
  assert.doesNotMatch(source, /runner\.temp \}\}\/core-rpc-docker-evidence\n/);
  assert.match(source, /services:\n      postgres:/);
  assert.match(source, /DATABASE_URL: postgres:\/\/postgres:postgres@127\.0\.0\.1:5432\/postgres/);
  assert.match(source, /working-directory: services\/extensions/);
  assert.doesNotMatch(source, /continue-on-error:|apps\/extensions-go/);
});

test('obsolete database upgrade tooling is not an alternate installation path', () => {
  for (const path of ['scripts/deploy-systemd.py', 'scripts/deploy-shared-postgres.py',
    'scripts/ci/qualify-go-migration-startup.sh', 'packaging/common/lmm-api/migration-compatibility.env']) {
    assert.throws(() => read(path), /ENOENT/, path);
  }
  assert.throws(() => read('.env.example'), /ENOENT/);
  for (const path of ['config/core.example.toml', 'config/extensions.example.toml']) {
    const sample = read(path);
    assert.match(sample, /version = 1/);
    assert.match(sample, /\/etc\/lmm\.conf\.d\/secrets\//);
    assert.doesNotMatch(sample, /SQL_DSN=|SESSION_SECRET=|SKIP_64BIT_QUOTA_SCHEMA_CHECK/);
  }
  for (const path of ['README.md', 'README_EN.md', 'docs/development.md']) {
    const text = read(path);
    assert.doesNotMatch(text, /Startup applies schema migrations by default|服务启动时默认执行数据库迁移/);
    assert.match(text, /init-db/);
  }
});

test('S11 snapshots are manual read-only sources, not automatic Git writers', () => {
  const source = workflow('s11-source-snapshot');
  assert.match(source, /on:\n  workflow_dispatch:/);
  assert.match(source, /permissions:\n  contents: read/);
  assert.match(source, /persist-credentials: false/);
  assert.match(source, /github\.repository == 'TokenNotIncluded\/lmm-api-source' && github\.ref == 'refs\/heads\/main'/);
  assert.match(source, /ref: \$\{\{ github\.sha \}\}/);
  assert.match(source, /git bundle create/);
  assert.match(source, /git submodule update --init -- apps\/cli/);
  assert.doesNotMatch(source, /contents: write|GH_TOKEN|github\.token|s11-rebase\.py|git push|refs\/pull\//);
});

test('S11 bundles restore the selected source and pinned CLI after main advances', (t) => {
  const directory = mkdtempSync(join(tmpdir(), 'lmm-snapshot-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const env = {
    ...gitFixtureEnvironment, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file',
    GIT_AUTHOR_NAME: 'Snapshot test', GIT_AUTHOR_EMAIL: 'snapshot@example.test',
    GIT_COMMITTER_NAME: 'Snapshot test', GIT_COMMITTER_EMAIL: 'snapshot@example.test',
  };
  const run = (cwd, command, args, extraEnv = {}) => execFileSync(command, args, {
    cwd, env: { ...env, ...extraEnv }, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'],
  }).trim();
  const git = (cwd, ...args) => run(cwd, 'git', args);
  const commit = (cwd, text) => {
    writeFileSync(join(cwd, 'README.md'), text);
    git(cwd, 'add', '.');
    git(cwd, 'commit', '-qm', text);
    return git(cwd, 'rev-parse', 'HEAD');
  };
  const cli = join(directory, 'cli');
  const source = join(directory, 'source');
  for (const repo of [cli, source]) {
    mkdirSync(repo);
    git(repo, 'init', '-q', '-b', 'main');
  }
  const cliRevision = commit(cli, 'pinned CLI');
  git(source, 'submodule', 'add', cli, 'apps/cli');
  const revision = commit(source, 'selected source');
  commit(cli, 'later CLI');
  const laterRevision = commit(source, 'later source');
  const checkout = join(directory, 'checkout');
  git(directory, 'clone', '-q', '--no-hardlinks', source, checkout);
  git(checkout, 'checkout', '-q', '--detach', revision);
  const snapshot = workflow('s11-source-snapshot');
  const init = snapshot.match(/        run: (git submodule update[^\n]+)/)[1];
  const script = snapshot.match(/        run: \|\n([\s\S]*?)(?=      - uses:)/)[1]
    .replace(/^          /gm, '');
  run(checkout, 'bash', ['-e', '-c', init]);
  const workflowEnv = { GITHUB_SHA: revision, GITHUB_REPOSITORY: 'TokenNotIncluded/lmm-api-source', RUNNER_TEMP: directory };
  run(checkout, 'bash', ['-e', '-c', script], workflowEnv);
  const output = join(directory, 's11-sources');
  run(output, 'sha256sum', ['--check', 'SHA256SUMS']);
  for (const [bundle, expected] of [['source', revision], ['cli', cliRevision]]) {
    const restored = join(directory, `restored-${bundle}`);
    git(directory, 'clone', '-q', join(output, `${bundle}.bundle`), restored);
    assert.equal(git(restored, 'rev-parse', 'HEAD'), expected);
  }
  assert.equal(readFileSync(join(output, 'refs.txt'), 'utf8'),
    `TokenNotIncluded/lmm-api-source ${revision}\n${cli} ${cliRevision}\n`);
  assert.throws(() => run(checkout, 'bash', ['-e', '-c', script], { ...workflowEnv, GITHUB_SHA: laterRevision }));
  git(join(checkout, 'apps/cli'), 'checkout', '-q', 'main');
  assert.throws(() => run(checkout, 'bash', ['-e', '-c', script], workflowEnv));
});

test('independent check failures do not skip the Go host required by Rust tests', () => {
  const source = workflow('core-protocol');
  assert.match(source, /name: Build actual Go host for Rust stream isolation\n        if: \$\{\{ !cancelled\(\) \}\}/);
  assert.match(source, /scripts\/test_core_rpc_fixture\.py -v/);
  assert.match(source, /tests\/payments\/test_verify_live_discovery\.py -v/);
  assert.doesNotMatch(source, /LMM_CORE_DATABASE_URL_SECRET_FILE|LMM_CORE_WORKER_THREADS/);
});

test('Core CI uses the same pinned PostgreSQL catalog as the RPC fixture', () => {
  const source = workflow('core-protocol');
  const ciImage = source.match(/services:\n      postgres:\n        image: (\S+)/)?.[1];
  const rpcImage = read('deployment/docker/compose.identity.yml')
    .match(/^    image: (postgres:\S+)/m)?.[1];
  assert.match(rpcImage ?? '', /^postgres:18-alpine@sha256:[a-f0-9]{64}$/);
  assert.equal(ciImage, rpcImage, 'reviewed catalog checks require the same PostgreSQL image');
});

test('native package uploads keep raw Alpine fixture output private', () => {
  const source = workflow('linux-packages');
  const upload = source.split('      - name: Preserve preview packages and scoped evidence, never publish\n')[1];
  const paths = upload?.match(/          path: \|\n((?:            .+\n)+)/)?.[1];
  assert.ok(paths, 'the native package artifact must enumerate reviewed paths');
  assert.deepEqual(paths.trim().split('\n').map((path) => path.trim()), [
    '${{ steps.packages.outputs.paths }}',
    'out/evidence/revision.txt',
    'out/evidence/nfpm-version.txt',
    'out/evidence/install-matrix.txt',
    'out/evidence/image-digests.txt',
    'out/evidence/extore-fixture-image.txt',
    'out/evidence/extore-native-alpine.json',
    'out/evidence/extore-native-alpine.json.package.json',
  ]);
  assert.ok(source.includes('--report "/receipts/$report" > "$staging/fixture.log" 2>&1'));
});

test('automatic Core checks partition every Rust target across independent runners', () => {
  const source = workflow('core-protocol');
  assert.match(source, /split_rust_targets:[\s\S]*?type: boolean\n        default: false/);
  assert.match(source, /fail-fast: false\n      matrix:\n        targets: .*inputs\.split_rust_targets && '\["unit","model-unit","model-responses","schema-identity","integration-1","integration-2","integration-3","integration-4"\]' \|\| '\["all"\]'/);
  assert.ok(source.includes('RUST_TARGET_GROUP: ${{ matrix.targets }}'));
  assert.match(source, /timeout-minutes: 150/);
  assert.doesNotMatch(source, /continue-on-error:/);
  const executor = source.match(/run_tests\(\) \{[\s\S]*?case "\$RUST_TARGET_GROUP" in[\s\S]*?\besac\b/)?.[0];
  assert.ok(executor);
  const selector = source.match(
    /      - name: Select Cargo integration targets\n[\s\S]*?        run: \|\n([\s\S]*?)(?=      - name:)/
  )?.[1];
  assert.ok(selector);
  assert.match(source, /mapfile -t targets < "\$RUNNER_TEMP\/core-integration-targets"\n\s+cargo test --locked --no-run "\$\{targets\[@\]\}"/);
  const metadata = {packages: [
    {name: 'lmm-core', targets: [
      ...['delta', 'alpha', 'gamma', 'beta', 'epsilon'].map(name => ({kind: ['test'], name})),
      {kind: ['lib'], name: 'lmm_core'},
      {kind: ['bench'], name: 'named_benchmark'},
    ]},
    {name: 'another-crate', targets: [{kind: ['test'], name: 'foreign'}]},
  ]};
  const stub = 'cargo() { if [[ $1 == metadata ]]; then printf "%s\\n" "$METADATA"; else printf "%s\\n" "$*"; fi; }\n';
  function calls(group, targets = metadata) {
    const temp = mkdtempSync(join(tmpdir(), 'lmm-ci-target-test-'));
    try {
      const select = group.startsWith('integration-') ? selector : '';
      return execFileSync('bash', ['-euo', 'pipefail', '-c', stub + select + executor], {
        env: {
          ...process.env, RUST_TARGET_GROUP: group, RUNNER_TEMP: temp,
          METADATA: JSON.stringify(targets),
        },
        stdio: ['ignore', 'pipe', 'pipe'],
        encoding: 'utf8',
      }).trim().split('\n').map(line => line.split(' --skip synthetic_live_proof')[0]);
    } finally {
      rmSync(temp, { recursive: true });
    }
  }
  const runner = 'test --locked --no-fail-fast ';
  assert.deepEqual(calls('all'), [runner + '--all-targets --']);
  assert.deepEqual(calls('unit'), [
    runner + '--lib -- --skip model_http:: --skip core_schema:: --skip identity::',
    runner + '--bins --examples --bench named_benchmark --',
  ]);
  assert.deepEqual(calls('unit', { packages: [{ name: 'lmm-core', targets: [] }] }).slice(1),
    [runner + '--bins --examples --']);
  assert.deepEqual(calls('model-unit'), [runner + '--lib -- model_http:: --skip model_http::responses_']);
  assert.deepEqual(calls('model-responses'), [runner + '--lib -- model_http::responses_']);
  assert.match(source, /cargo build --locked --bin lmm-core\n\s+cargo test --locked --lib --no-run/);
  assert.deepEqual(calls('schema-identity'), [runner + '--lib -- core_schema:: identity:: --skip model_http::']);
  assert.deepEqual(calls('integration-1'), [runner + '--test alpha --test gamma --']);
  assert.deepEqual(calls('integration-2'), [runner + '--test beta --']);
  assert.deepEqual(calls('integration-3'), [runner + '--test delta --']);
  assert.deepEqual(calls('integration-4'), [runner + '--test epsilon --']);
  assert.throws(() => calls('unknown'), { status: 1 });
});

test('Core CI freezes the process binary and private root before running all Rust tests', () => {
  const source = workflow('core-protocol');
  const block = source.match(/      - name: Rust checks and real database tests\n([\s\S]*?)(?=      - name:)/)?.[1];
  assert.ok(block, 'the Rust database test step must exist');
  const compileStep = source.indexOf('id: rust_test_compile');
  const compile = source.indexOf('cargo test --locked --all-targets --no-run --message-format=json');
  const upload = source.indexOf('name: Preserve compiled commerce acceptance driver');
  const executeStep = source.indexOf('name: Rust checks and real database tests');
  assert.ok(compileStep >= 0 && compile > compileStep && upload > compile && executeStep > upload,
    'compile once and retain its driver before executing the database tests');
  assert.equal(source.split('cargo test --locked --all-targets --no-run').length - 1, 1);
  const driver = source.match(/      - name: Preserve compiled commerce acceptance driver\n([\s\S]*?)(?=      - name:)/)?.[1];
  assert.ok(driver, 'the compiled acceptance driver must be preserved');
  assert.match(block, /if: \$\{\{ !cancelled\(\) && steps\.rust_test_compile\.outcome == 'success' \}\}/);
  assert.match(driver, /if: \$\{\{ !cancelled\(\) && \(matrix\.targets == 'all' \|\| matrix\.targets == 'unit'\) && steps\.rust_test_compile\.outcome == 'success' \}\}/);
  assert.match(driver, /name: commerce-acceptance-driver-\$\{\{ github\.sha \}\}-\$\{\{ github\.run_id \}\}\n/);
  assert.match(source, /name: Preserve source before checks\n\s+if: \$\{\{ matrix\.targets == 'all' \|\| matrix\.targets == 'unit' \}\}/);
  const binary = block.indexOf('LMM_MODEL_PROCESS_CORE_BIN="$(realpath target/debug/lmm-core)"');
  const hash = block.indexOf('LMM_MODEL_PROCESS_CORE_SHA256="$(sha256sum "$LMM_MODEL_PROCESS_CORE_BIN"');
  const root = block.indexOf('LMM_MODEL_PROCESS_ROOT="$(mktemp -d "$process_evidence_root/model-XXXXXXXX")"');
  const exported = block.indexOf('export LMM_MODEL_PROCESS_CORE_BIN LMM_MODEL_PROCESS_CORE_SHA256 LMM_MODEL_PROCESS_ROOT');
  const execute = block.indexOf('cargo test --locked --no-fail-fast "$@"');
  assert.ok(binary >= 0 && hash > binary && root > hash
    && exported > root && execute > exported, 'freeze inputs after compiling, before executing');
  // Each omitted Cargo invocation still has a required owning runner.
  assert.deepEqual(
    [...block.matchAll(/--skip (\w+)(?=\s)/g)].map((match) => match[1]),
    ['synthetic_live_proof_crosses_core_go_processes_and_posts_once',
      'offline_financial_import_contracts_and_postgres'],
  );
  assert.doesNotMatch(block, /--exclude|--ignored|continue-on-error|\|\|\s*true/);
});

test('Core CI isolates independent process evidence and cluster-wide role snapshots', () => {
  const source = workflow('core-protocol');
  assert.match(source, /RUST_TEST_THREADS: "1"/);
  const block = source.match(/      - name: Rust checks and real database tests\n([\s\S]*?)(?=      - name:)/)?.[1];
  assert.ok(block);
  const execute = block.indexOf('cargo test --locked --no-fail-fast "$@"');
  for (const name of [
    'LMM_CONTINUITY_DEV_ARTIFACTS',
    'COMPACT_INDEPENDENT_ARTIFACTS',
    'RESPONSES_CONTINUITY_INDEPENDENT_ARTIFACTS',
    'RESPONSES_CONTINUITY_REVIEW_ARTIFACTS',
    'LMM_WS_ARTIFACTS',
  ]) {
    const assigned = block.indexOf(name + '="$(mktemp -d "$process_evidence_root/');
    const exported = block.split('\n').findIndex((line) =>
      line.trimStart().startsWith('export ') && line.split(/\s+/).includes(name));
    assert.ok(assigned >= 0 && assigned < execute, name + ' requires a new private directory before execution');
    assert.ok(exported >= 0, name + ' must reach the test process');
  }
  assert.match(block, /LMM_WS_CORE_BINARY="\$LMM_MODEL_PROCESS_CORE_BIN"/);
  assert.match(block, /export LMM_WS_ARTIFACTS LMM_WS_CORE_BINARY/);
  assert.match(block, /process_evidence_root="\$\(mktemp -d "\$RUNNER_TEMP\/core-process-evidence-XXXXXXXX"\)"/);
  assert.match(source, /\$\{\{ runner.temp \}\}\/core-process-evidence-\*/);
});

test('preserved migration executors build only on integration or full test hosts', () => {
  const source = workflow('core-protocol');
  assert.match(source, /name: Build the preserved migration executors\n        if: \$\{\{ !cancelled\(\) && \(startsWith\(matrix.targets, 'integration'\) \|\| matrix.targets == 'all'\) \}\}/);
  assert.match(source, /integration-\[1-4\]\)/);
  assert.match(source, /run_tests --all-targets --/);
});

test('externally owned payment live case remains required through its real runner', () => {
  const source = workflow('core-protocol');
  assert.match(source, /--skip synthetic_live_proof_crosses_core_go_processes_and_posts_once/);
  assert.match(source, /name: Synthetic payment live process through owning runner\n        if: \$\{\{ !cancelled\(\) && \(matrix.targets == 'integration-1' \|\| matrix.targets == 'all'\) \}\}\n        run: python3 -B scripts\/ci-payment-live.py/);
  assert.match(source, /payment-live\/evidence\/result.json/);
  const script = read('scripts/ci-payment-live.py');
  assert.ok(script.indexOf('frozen = live.v.sources()') < script.indexOf('build = subprocess.run('));
  assert.match(script, /"--test", "payments_live_process", "--no-run", "--message-format=json"/);
  assert.match(script, /len\(matches\) != 1/);
  assert.match(script, /live.v.sources\(\) != frozen/);
  assert.match(script, /"--compiled-inputs", str\(inputs\)/);
  assert.match(script, /live.sha\(binary\)/);
  assert.match(script, /return subprocess.run\(command, cwd=ROOT, env=env\).returncode/);
  assert.doesNotMatch(script, /LMM_LIVE_PROCESS_ROOT|owner.json/);
});

test('financial and RPC acceptance run independently and remain required in Core CI', () => {
  const source = workflow('core-protocol');
  const [rust, remainder] = source.split('\n  financial-import:\n');
  const [financial, rpc] = remainder.split('\n  rpc:\n');
  assert.doesNotMatch(rust, /run: .*test-core-rpc-docker/);
  for (const job of [financial, rpc]) {
    assert.match(job, /runs-on: ubuntu-24\.04/);
    assert.doesNotMatch(job, /(?:^    (?:if|needs):|continue-on-error:)/m);
  }
  assert.match(financial, /bash scripts\/install-ci-postgresql18\.sh/);
  assert.match(financial, /docker pull postgres:18-alpine@sha256:[0-9a-f]{64}/);
  assert.match(financial, /LMM_FINANCIAL_PG_TEST: "1"\n        run: python3 -B scripts\/run-reviewed-financial-tests\.py/);
  assert.match(rpc, /packages\/extore[\s\S]*python3 -B scripts\/check-extore-plugin-pin\.py/);
  assert.match(rpc, /run: \|[\s\S]*python3 -B scripts\/test-core-rpc-docker\.py/);
});
