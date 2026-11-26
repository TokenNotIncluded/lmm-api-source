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
test('preview workflows stay read-only and source publication never grants server access', () => {
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
    'core-protocol': ['apps/cli', 'packages/extore'],
    deploy: ['packages/extore'],
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
  assert.doesNotMatch(workflow('ci'), /service-unavailable\.test\.mjs/);
});


test('automatic checks and explicit source-release requests have bounded triggers', () => {
  for (const file of readdirSync(new URL('.github/workflows/', root)).filter((name) => /\.ya?ml$/.test(name))) {
    const source = read(`.github/workflows/${file}`);
    const block = source.match(/^on:\n((?:[ \t].*\n|\n)*)/m);
    assert.ok(block, `${file}: event block is required`);
    const events = [...block[1].matchAll(/^  ([\w-]+):/gm)].map((match) => match[1]);
    if (file === 'ci.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'push', 'pull_request']);
      assert.match(block[1], /pull_request:\n    branches: \[main\]/);
      assert.match(block[1], /push:\n    branches: \[main\]/);
      assert.doesNotMatch(block[1], /paths(?:-ignore)?:|tags:/);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|pull_request_target|environment: production/);
    } else if (file === 'core-protocol.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'workflow_call']);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|pull_request_target|environment: production/);
    } else if (file === 'source-release.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'push']);
      assert.match(block[1], /push:\n    branches: \[main\]\n    paths: \[\.github\/source-release-request\.json\]/);
      assert.doesNotMatch(source, /pull_request_target|id-token: write|environment: production/);
    } else if (file === 'deploy.yml') {
      assert.deepEqual(events, ['workflow_dispatch', 'push', 'workflow_run']);
      assert.match(block[1], /tags: \['v\*', 'shadow-v\*'\]/);
      assert.match(block[1], /workflow_run:\n    workflows: \[CI\]\n    types: \[completed\]\n    branches: \[main\]/);
      assert.match(source, /github\.event\.workflow_run\.conclusion == 'success'/);
      assert.match(source, /github\.event\.workflow_run\.event == 'push'/);
      assert.match(source, /github\.event\.workflow_run\.head_repository\.full_name == github\.repository/);
      assert.match(source, /Recheck automatic deployment against current main/);
      assert.doesNotMatch(block[1], /pull_request|schedule:/);
      assert.match(source, /DEPLOY_TARGETS: \$\{\{ vars\.DEPLOY_TARGETS \}\}/);
      assert.match(source, /git merge-base --is-ancestor "\$revision" origin\/main/);
      assert.match(source, /needs: \[plan, ci-gate, build\]/);
      assert.match(source, /cancel-in-progress: false/);
      assert.match(source, /ANSIBLE_HOST_KEY_CHECKING: 'True'/);
      assert.doesNotMatch(source, /ArchDmit|DmitUbuntu|native\.api\.lmm\.best|contents: write|id-token: write/);
    } else if (file === 'layout-reflow-review.yml') {
      // The existing UI fixture workflow is automatic and read-only.
      assert.deepEqual(events, ['workflow_dispatch', 'push', 'pull_request']);
      assert.match(block[1], /push:\n    branches: \[main\]\n    paths:/);
      assert.match(source, /permissions:\n  contents: read/);
      assert.doesNotMatch(source, /secrets\.|contents: write|pull_request_target|environment: production/);
      assert.match(source, /127\.0\.0\.1:4174/);
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

test('deployment selects only commit objects from main and plans from that exact checkout', (t) => {
  const workflowSource = workflow('deploy');
  const plan = workflowSource.split('  ci-gate:\n')[0];
  assert.match(plan, /source_revision:\n\s+description:[^\n]+\n\s+type: string/);
  assert.match(plan, /fetch-depth: 0/);
  assert.match(plan, /REQUESTED_REVISION: \$\{\{ inputs\.source_revision \}\}/);
  assert.match(plan, /revision: \$\{\{ steps\.source\.outputs\.revision \}\}/);
  const script = plan.match(/id: source\n[\s\S]*?        run: \|\n([\s\S]*?)      - name: Select targets/)[1];
  const temp = mkdtempSync(join(tmpdir(), 'deployment-source-'));
  t.after(() => rmSync(temp, { recursive: true, force: true }));
  const run = (cmd, args, extra = {}) => execFileSync(cmd, args, {
    cwd: temp, encoding: 'utf8', stdio: 'pipe', env: {
      ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1',
      GIT_AUTHOR_NAME: 'Fixture', GIT_COMMITTER_NAME: 'Fixture',
      GIT_AUTHOR_EMAIL: 'fixture@example.test', GIT_COMMITTER_EMAIL: 'fixture@example.test', ...extra,
    },
  }).trim();
  const git = (...args) => run('git', args);
  git('init', '-q');
  git('commit', '--allow-empty', '-qm', 'ancestor');
  const ancestor = git('rev-parse', 'HEAD');
  git('commit', '--allow-empty', '-qm', 'latest');
  const tip = git('rev-parse', 'HEAD');
  git('update-ref', 'refs/remotes/origin/main', tip);
  const tree = git('rev-parse', 'HEAD^{tree}');
  const unrelated = git('commit-tree', tree, '-m', 'outside main history');
  git('tag', '-a', '-m', 'annotated fixture', 'fixture', ancestor);
  const tagObject = git('rev-parse', 'refs/tags/fixture');
  const output = join(temp, 'workflow-output');
  const select = (revision, event = 'workflow_dispatch', ref = 'refs/heads/main', ciRevision = '') => {
    git('checkout', '-q', '--detach', tip);
    writeFileSync(output, '');
    run('bash', ['-eo', 'pipefail', '-c', script], {
      GITHUB_EVENT_NAME: event, GITHUB_REF: ref, GITHUB_OUTPUT: output, REQUESTED_REVISION: revision, CI_REVISION: ciRevision,
    });
    return [git('rev-parse', 'HEAD'), readFileSync(output, 'utf8')];
  };
  for (const revision of [ancestor, ancestor.toUpperCase(), tip, '']) {
    const chosen = revision.toLowerCase() || tip;
    assert.deepEqual(select(revision), [chosen, 'revision=' + chosen + '\n']);
  }
  assert.deepEqual(select('', 'push', 'refs/tags/shadow-v1.2.3'), [tip, 'revision=' + tip + '\n']);
  assert.deepEqual(select('', 'workflow_run', 'refs/heads/main', tip), [tip, 'revision=' + tip + '\n']);
  for (const candidate of [ancestor, unrelated, '']) {
    assert.throws(() => select('', 'workflow_run', 'refs/heads/main', candidate));
    assert.equal(readFileSync(output, 'utf8'), '');
  }

  for (const args of [
    [unrelated], [tree], [tagObject], ['0'.repeat(40)], [ancestor.slice(0, 12)], ['main'],
    ['HEAD~1'], [' ' + ancestor], [ancestor + '\n'], [ancestor, 'push', 'refs/tags/shadow-v1'],
    [ancestor, 'workflow_dispatch', 'refs/tags/v1'], [ancestor, 'workflow_dispatch', 'refs/heads/other'],
  ]) {
    assert.throws(() => select(...args), 'admitted ' + args);
    assert.equal(readFileSync(output, 'utf8'), '');
    assert.equal(git('rev-parse', 'HEAD'), tip);
  }
  git('update-ref', 'refs/remotes/origin/main', ancestor);
  assert.throws(() => select('', 'push', 'refs/tags/v1'));
});

test('configured deployment reads successful CI for the exact planned source before building', () => {
  const source = workflow('deploy');
  const gate = source.split('  ci-gate:\n')[1].split('  build:\n')[0];
  const build = source.split('  build:\n')[1].split('  deploy:\n')[0];
  assert.match(gate, /needs: plan/);
  assert.match(gate, /permissions:\n      contents: read\n      actions: read/);
  assert.match(gate, /ref: \$\{\{ needs\.plan\.outputs\.revision \}\}/);
  assert.match(gate, /SOURCE_REVISION: \$\{\{ needs\.plan\.outputs\.revision \}\}/);
  assert.match(gate, /GH_TOKEN: \$\{\{ github\.token \}\}/);
  assert.match(gate, /python3 -B scripts\/test-verify-deployment-ci\.py/);
  assert.match(gate, /python3 -B scripts\/verify-deployment-ci\.py --revision "\$SOURCE_REVISION"/);
  assert.doesNotMatch(gate, /contents: write|actions: write|continue-on-error|always\(\)|cargo|go test/);
  assert.match(build, /needs: \[plan, ci-gate\]/);
  assert.match(gate, /if: inputs\.source_revision != ''\n        run: git restore --source="\$GITHUB_SHA" -- scripts\/verify-deployment-ci\.py scripts\/test-verify-deployment-ci\.py/);
  const deploy = source.split('  deploy:\n')[1];
  for (const job of [build, deploy]) {
    assert.match(job, /ref: \$\{\{ needs\.plan\.outputs\.revision \}\}/);
    assert.doesNotMatch(job, /GITHUB_SHA|github\.sha|inputs\.source_revision/);
  }
  assert.match(build, /name: web-\$\{\{ needs\.plan\.outputs\.revision \}\}-\$\{\{ needs\.ci-gate\.outputs\.run-id \}\}/);
  assert.match(build, /run-id: \$\{\{ needs\.ci-gate\.outputs\.run-id \}\}/);
  assert.match(build, /github-token: \$\{\{ github\.token \}\}/);
  assert.doesNotMatch(build, /bun run build:web/);
  assert.match(build, /--revision "\$SOURCE_REVISION"/);
  assert.match(deploy, /DEPLOY_REVISION: \$\{\{ needs\.plan\.outputs\.revision \}\}/);
  const boundaries = workflow('core-protocol')
    .match(/name: Workflow and source boundaries\n        run: \|\n([\s\S]*?)(?=      - name:)/)?.[1];
  assert.match(boundaries ?? '', /python3 -B scripts\/test-verify-deployment-ci\.py/);
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

test('CI and manual qualification reuse the exact core and extension verification pipeline', () => {
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

test('the CI gate cannot pass when an implemented component fails or is skipped', () => {
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

test('continuous CI bounds duplicate work and preserves only checked web assets', () => {
  const source = workflow('ci');
  const revision = "${{ github.event_name == 'workflow_dispatch' && github.sha || github.ref }}";
  assert.ok(source.includes('group: ci-${{ github.event_name }}-' + revision + '\n  cancel-in-progress: true'));
  const protocol = workflow('core-protocol');
  assert.ok(protocol.includes('group: microkernel-${{ github.workflow }}-${{ github.event_name }}-' + revision + '\n  cancel-in-progress: true'));
  const web = source.split('  web:\n')[1].split('  translations:\n')[0];
  assert.match(web, /timeout-minutes: 20/);
  for (const command of ['format:check', 'copyright:check', 'lint', 'typecheck', 'test', 'build', 'bundle:check']) {
    assert.ok(web.includes(`bun run --filter @lmm/web ${command}\n`), command);
  }
  assert.equal((web.match(/bun run --filter @lmm\/web test\n/g) ?? []).length, 1);
  assert.doesNotMatch(web, /bun test |continue-on-error:|if:.*(?:always|failure)/);
  const build = web.indexOf('bun run --filter @lmm/web build\n');
  const check = web.indexOf('bun run --filter @lmm/web bundle:check\n');
  const upload = web.indexOf('uses: actions/upload-artifact@');
  assert.ok(build >= 0 && check > build && upload > check);
  assert.match(web, /path: apps\/web\/dist\/\n          include-hidden-files: true\n          if-no-files-found: error\n          retention-days: 3/);
  assert.match(source, /github\.event\.pull_request\.base\.sha/);
  assert.match(source, /github\.event\.before/);
  assert.match(source, /node scripts\/check-i18n\.mjs --base "\$BASE_REF" --merge-base/);
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
  assert.match(source, /tests\/payments\/test_verify_live_discovery\.py -v/);
  assert.doesNotMatch(source, /LMM_CORE_DATABASE_URL_SECRET_FILE|LMM_CORE_WORKER_THREADS/);
  // All main pushes and PRs reach this suite through CI, including workflow-only edits.
  assert.match(workflow('ci'), /uses: \.\/\.github\/workflows\/core-protocol\.yml/);
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

test('automatic Core checks run both Cargo target groups on independent runners', () => {
  const source = workflow('core-protocol');
  for (const caller of ['ci', 'mk-01-identity']) {
    assert.match(workflow(caller), /uses: \.\/\.github\/workflows\/core-protocol\.yml\n    with:\n      split_rust_targets: true/);
  }
  assert.match(source, /split_rust_targets:[\s\S]*?type: boolean\n        default: false/);
  assert.match(source, /fail-fast: false\n      matrix:\n        targets: .*inputs\.split_rust_targets && '\["unit","integration"\]' \|\| '\["all"\]'/);
  assert.ok(source.includes('RUST_TARGET_GROUP: ${{ matrix.targets }}'));
  assert.match(source, /timeout-minutes: 150/);
  assert.doesNotMatch(source, /continue-on-error:/);
  const selector = source.match(/case "\$RUST_TARGET_GROUP" in[\s\S]*?\besac\b/)?.[0];
  assert.ok(selector);
  function flags(group) {
    return execFileSync('bash', ['-c', selector + '\nprintf "%s\\n" "${targets[@]}"'], {
      env: { ...process.env, RUST_TARGET_GROUP: group },
      stdio: ['ignore', 'pipe', 'pipe'],
      encoding: 'utf8',
    }).trim().split('\n');
  }
  assert.deepEqual(flags('all'), ['--all-targets']);
  assert.deepEqual(flags('unit'), ['--lib', '--bins', '--examples', '--benches']);
  assert.deepEqual(flags('integration'), ['--test', '*']);
  assert.throws(() => flags('unknown'), { status: 1 });
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
  for (const guarded of [driver, block]) {
    assert.match(guarded, /if: \$\{\{ !cancelled\(\) && steps\.rust_test_compile\.outcome == 'success' \}\}/);
  }
  const binary = block.indexOf('LMM_MODEL_PROCESS_CORE_BIN="$(realpath target/debug/lmm-core)"');
  const hash = block.indexOf('LMM_MODEL_PROCESS_CORE_SHA256="$(sha256sum "$LMM_MODEL_PROCESS_CORE_BIN"');
  const root = block.indexOf('LMM_MODEL_PROCESS_ROOT="$(mktemp -d "$process_evidence_root/model-XXXXXXXX")"');
  const exported = block.indexOf('export LMM_MODEL_PROCESS_CORE_BIN LMM_MODEL_PROCESS_CORE_SHA256 LMM_MODEL_PROCESS_ROOT');
  const execute = block.indexOf('cargo test --locked "${targets[@]}" --no-fail-fast');
  assert.ok(binary >= 0 && hash > binary && root > hash
    && exported > root && execute > exported, 'freeze inputs after compiling, before executing');
  // The separately required owning-runner step covers exactly this one case.
  const routed = ' -- --skip synthetic_live_proof_crosses_core_go_processes_and_posts_once';
  assert.equal(block.split(routed).length - 1, 1);
  assert.doesNotMatch(block.replace(routed, ''), /--skip|--exclude|--ignored|continue-on-error|\|\|\s*true/);
});

test('Core CI isolates independent process evidence and cluster-wide role snapshots', () => {
  const source = workflow('core-protocol');
  assert.match(source, /RUST_TEST_THREADS: "1"/);
  const block = source.match(/      - name: Rust checks and real database tests\n([\s\S]*?)(?=      - name:)/)?.[1];
  assert.ok(block);
  const execute = block.indexOf('cargo test --locked "${targets[@]}" --no-fail-fast');
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
