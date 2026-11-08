import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

const root = new URL('../', import.meta.url);
const read = (path) => readFileSync(new URL(path, root), 'utf8');
const workflow = (name) => read(`.github/workflows/${name}.yml`);

// Source contracts complement actionlint and runtime tests. They do not assert
// that unimplemented business modules are ready or that release is authorized.
test('current workflows preserve preview artifacts without publishing or server access', () => {
  const files = readdirSync(new URL('.github/workflows/', root))
    .filter((name) => /\.ya?ml$/.test(name));
  for (const name of ['ci.yml', 'core-protocol.yml', 'microservice-distribution.yml', 'server-release-qualification.yml']) {
    assert.ok(files.includes(name), `missing workflow: ${name}`);
  }
  assert.ok(!files.includes('server-ops.yml'));
  for (const file of files) {
    const source = read(`.github/workflows/${file}`);
    assert.doesNotMatch(source, /PRODUCTION_SSH|production-auto-deploy|deploy-production|server-ops|inputs\.deploy/);
    assert.doesNotMatch(source, /environment:\s*production/);
    assert.doesNotMatch(source, /^\s*(?:run:\s*)?(?:ssh|scp|rsync)\s/m);
    assert.doesNotMatch(source, /contents: write|id-token: write|gh release\s/);
  }
  for (const path of ['.github/actions/deploy-production/action.yml', '.github/server-ops-343-request.json', 'scripts/auto-deploy-production-release.sh', 'scripts/server-ops.py']) {
    assert.throws(() => read(path), /ENOENT/);
  }
  assert.throws(() => workflow('release-go'), /ENOENT/);
  assert.throws(() => workflow('release-web'), /ENOENT/);
  assert.throws(() => workflow('deploy-web-frontend'), /ENOENT/);
  assert.match(workflow('server-release-qualification'), /uses: \.\/\.github\/workflows\/core-protocol\.yml/);
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

test('CLI consumers initialize only the pinned CLI submodule', () => {
  for (const name of ['lmm', 'microservice-distribution', 'linux-packages', 'core-protocol']) {
    const source = workflow(name);
    const commands = [...source.matchAll(/git submodule update[^\n]*/g)].map((match) => match[0]);
    assert.equal(commands.length, name === 'microservice-distribution' ? 2 : 1, name);
    for (const command of commands) {
      assert.equal(command, 'git submodule update --init --depth 1 -- apps/cli', name);
    }
    assert.doesNotMatch(source, /submodules:\s*(?:true|recursive)/);
  }
  assert.match(workflow('lmm'), /name: Initialize the pinned CLI only\n        working-directory: \.\n/);
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
  assert.doesNotMatch(workflow('ci'), /service-unavailable\.test\.mjs/);
});


test('only the isolated microkernel checks may run automatically', () => {
  for (const file of readdirSync(new URL('.github/workflows/', root)).filter((name) => /\.ya?ml$/.test(name))) {
    const source = read(`.github/workflows/${file}`);
    const block = source.match(/^on:\n((?:[ \t].*\n|\n)*)/m);
    assert.ok(block, `${file}: event block is required`);
    const events = [...block[1].matchAll(/^  ([\w-]+):/gm)].map((match) => match[1]);
    if (file === 'core-protocol.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'workflow_call', 'pull_request', 'push']);
      assert.match(block[1], /pull_request:\n    branches: \[main\]/);
      assert.match(block[1], /push:\n    branches: \[main\]/);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|pull_request_target|environment: production/);
    } else if (file === 'linux-packages.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'workflow_call', 'pull_request']);
      assert.match(block[1], /pull_request:\n    branches: \[main\]/);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|id-token: write|pull_request_target|gh release|environment: production/);
      assert.match(source, /packaging\/linux\/test_native\.py/);
    } else if (file === 'microservice-distribution.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'workflow_call']);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|id-token: write|pull_request_target|gh release|environment: production/);
      assert.match(source, /scripts\/build-distribution\.py verify/);
      assert.match(source, /scripts\/test-legal-process\.py/);
    } else if (file === 'mk10-payments.yml') {
      // Payment tests use disposable PostgreSQL, never a channel credential.
      assert.deepEqual(events, ['pull_request', 'push']);
      assert.match(block[1], /pull_request:\n    branches: \[main\]/);
      assert.match(block[1], /push:\n    branches: \[main\]/);
      assert.match(block[1], /services\/extensions\/internal\/modules\/payments\/\*\*/);
      assert.match(source, /permissions:\n  contents: read/);
      assert.match(source, /services:\n      postgres:/);
      assert.doesNotMatch(source, /secrets\.|contents: write|pull_request_target|environment: production/);
    } else if (file === 'mk-09-assistant-promotions.yml') {
      // The merged MK-09 module retains isolated, read-only regression checks.
      assert.deepEqual(events, ['push', 'pull_request', 'workflow_dispatch']);
      assert.match(block[1], /push:\n    branches: \[main\]\n    paths:/);
      assert.match(block[1], /pull_request:\n    branches: \[main\]/);
      assert.match(block[1], /services\/extensions\/internal\/modules\/assistant\/\*\*/);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|pull_request_target|environment: production/);
    } else if (file === 'mk-01-identity.yml') {
      // MK-01 uses disposable databases for changes to main and its PRs.
      assert.deepEqual(events, ['push', 'pull_request']);
      assert.match(block[1], /push:\n    branches: \[main\]\n    paths:/);
      assert.match(block[1], /pull_request:\n    branches: \[main\]/);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|pull_request_target|environment: production/);
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

test('manual checks reuse the exact core and extension verification pipeline', () => {
  for (const name of ['ci', 'server-release-qualification']) {
    const source = workflow(name);
    assert.match(source, /uses: \.\/\.github\/workflows\/core-protocol\.yml/);
    assert.doesNotMatch(source, /qualify-go-migration-startup|credit-balance-rebase|\.\/model|\.\/relaykit/);
  }
  const source = workflow('core-protocol');
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

test('the full manual gate cannot pass when an implemented component fails or is skipped', () => {
  const source = workflow('ci');
  assert.match(source, /needs: \[microkernel, web, translations\]/);
  assert.match(source, /if: \$\{\{ always\(\) \}\}/);
  for (const name of ['KERNEL_RESULT', 'WEB_RESULT', 'TRANSLATION_RESULT']) {
    assert.ok(source.includes(`test "$${name}" = success`));
  }
  assert.match(source, /name: Translation regression check/);
  assert.match(source, /bun install --frozen-lockfile/);
  assert.doesNotMatch(source, /continue-on-error:/);
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
    ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file',
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
  assert.doesNotMatch(source, /LMM_CORE_DATABASE_URL_SECRET_FILE|LMM_CORE_WORKER_THREADS/);
  assert.match(source, /pull_request:[\s\S]*?paths:[\s\S]*?\.github\/workflows\/\*\.yml/);
});


test('Core CI uses the same pinned PostgreSQL catalog as the RPC fixture', () => {
  const source = workflow('core-protocol');
  const ciImage = source.match(/services:\n      postgres:\n        image: (\S+)/)?.[1];
  const rpcImage = read('deployment/docker/compose.identity.yml')
    .match(/^    image: (postgres:\S+)/m)?.[1];
  assert.match(rpcImage ?? '', /^postgres:18-alpine@sha256:[a-f0-9]{64}$/);
  assert.equal(ciImage, rpcImage, 'reviewed catalog checks require the same PostgreSQL image');
});


test('Core CI freezes the process binary and private root before running all Rust tests', () => {
  const source = workflow('core-protocol');
  const block = source.match(/      - name: Rust checks and real database tests\n([\s\S]*?)(?=      - name:)/)?.[1];
  assert.ok(block, 'the Rust database test step must exist');
  const compile = block.indexOf('cargo test --locked --all-targets --no-run');
  const binary = block.indexOf('LMM_MODEL_PROCESS_CORE_BIN="$(realpath target/debug/lmm-core)"');
  const hash = block.indexOf('LMM_MODEL_PROCESS_CORE_SHA256="$(sha256sum "$LMM_MODEL_PROCESS_CORE_BIN"');
  const root = block.indexOf('LMM_MODEL_PROCESS_ROOT="$(mktemp -d /tmp/lmm-model-XXXXXXXX)"');
  const exported = block.indexOf('export LMM_MODEL_PROCESS_CORE_BIN LMM_MODEL_PROCESS_CORE_SHA256 LMM_MODEL_PROCESS_ROOT');
  const execute = block.indexOf('cargo test --locked --all-targets --no-fail-fast');
  assert.ok(compile >= 0 && binary > compile && hash > binary && root > hash
    && exported > root && execute > exported, 'freeze inputs after compiling, before executing');
  // The separately required owning-runner step covers exactly this one case.
  const routed = ' -- --skip synthetic_live_proof_crosses_core_go_processes_and_posts_once';
  assert.equal(block.split(routed).length - 1, 1);
  assert.doesNotMatch(block.replace(routed, ''), /--skip|--exclude|--ignored|continue-on-error|\|\|\s*true/);
});

test('externally owned payment live case remains required through its real runner', () => {
  const source = workflow('core-protocol');
  assert.match(source, /--no-fail-fast -- --skip synthetic_live_proof_crosses_core_go_processes_and_posts_once/);
  assert.match(source, /name: Synthetic payment live process through owning runner\n        if: \$\{\{ !cancelled\(\) \}\}\n        run: python3 -B scripts\/ci-payment-live.py/);
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
