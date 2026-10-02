// Diagnostic only: no prompts, real credentials, installs or provider calls.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

const packageRoot = process.argv[2];
assert(packageRoot, 'Usage: node scripts/probe-pi-rpc.mjs <installed-pi-package-directory>');
const manifest = JSON.parse(readFileSync(join(packageRoot, 'package.json'), 'utf8'));
assert.equal(manifest.name, '@earendil-works/pi-coding-agent');
const cli = resolve(packageRoot, manifest.bin.pi);
const root = mkdtempSync(join(tmpdir(), 'ariel-pi-rpc-'));
const agentDir = join(root, 'agent');
mkdirSync(agentDir);
writeFileSync(join(agentDir, 'models.json'), JSON.stringify({ providers: {
  'ariel-fixture': {
    baseUrl: 'http://127.0.0.1:9/v1', api: 'openai-completions',
    apiKey: 'synthetic-not-a-credential', models: [{ id: 'fixture-a' }, { id: 'fixture-b' }],
  },
} }));
const env = {};
for (const key of ['PATH', 'Path', 'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT', 'TEMP', 'TMP']) {
  if (process.env[key] !== undefined) env[key] = process.env[key];
}
Object.assign(env, {
  PI_CODING_AGENT_DIR: agentDir, PI_OFFLINE: '1', PI_SKIP_VERSION_CHECK: '1',
  PI_TELEMETRY: '0', USERPROFILE: root, HOME: root, APPDATA: root, LOCALAPPDATA: root,
});
const args = [cli, '--mode', 'rpc', '--no-session', '--no-tools', '--no-extensions',
  '--no-skills', '--no-prompt-templates', '--no-themes', '--no-context-files', '--no-approve', '--offline'];
const child = spawn(process.execPath, args, { cwd: root, env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
const pending = new Map();
let buffer = Buffer.alloc(0);
let sequence = 0;
let stderrBytes = 0;
let fatal;
let closed = false;
const fail = (error) => {
  fatal ??= error;
  for (const item of pending.values()) item.reject(error);
  pending.clear();
};
child.on('error', fail);
child.stdin.on('error', fail);
child.stderr.on('data', chunk => { stderrBytes += chunk.length; });
child.stdout.on('data', chunk => {
  buffer = Buffer.concat([buffer, chunk]);
  if (buffer.length > 4 * 1024 * 1024) return fail(new Error('RPC buffer limit exceeded'));
  let newline;
  while ((newline = buffer.indexOf(10)) !== -1) {
    const line = buffer.subarray(0, newline).toString('utf8').replace(/\r$/, '');
    buffer = buffer.subarray(newline + 1);
    if (!line) continue;
    try {
      const record = JSON.parse(line);
      if (record.type === 'response' && pending.has(record.id)) {
        const waiter = pending.get(record.id);
        pending.delete(record.id);
        waiter.resolve(record);
      }
    } catch { fail(new Error('Invalid JSONL on stdout')); }
  }
});
const exit = new Promise(resolveExit => child.on('close', (code, signal) => {
  closed = true;
  if (pending.size) fail(new Error('Pi exited with pending RPC commands'));
  resolveExit({ code, signal });
}));
async function rpc(type, fields = {}) {
  if (fatal) throw fatal;
  assert(!closed, 'Pi already exited');
  const id = `probe-${++sequence}`;
  let timer;
  try {
    const response = new Promise((resolveResponse, reject) => {
      pending.set(id, { resolve: resolveResponse, reject });
      timer = setTimeout(() => { pending.delete(id); reject(new Error(`RPC timeout: ${type}`)); }, 15000);
    });
    child.stdin.write(JSON.stringify({ id, type, ...fields }) + '\n');
    const result = await response;
    assert.equal(result.id, id);
    assert.equal(result.command, type);
    return result;
  } finally { clearTimeout(timer); }
}
async function waitExit(ms) {
  let timer;
  try {
    return await Promise.race([exit, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Pi shutdown timeout')), ms);
    })]);
  } finally { clearTimeout(timer); }
}
try {
  const catalog = await rpc('get_available_models');
  assert.equal(catalog.success, true);
  const models = catalog.data.models.filter(model => model.provider === 'ariel-fixture');
  assert.deepEqual(models.map(model => model.id).sort(), ['fixture-a', 'fixture-b']);
  const selected = await rpc('set_model', { provider: 'ariel-fixture', modelId: 'fixture-a' });
  assert.equal(selected.success, true);
  assert.equal(selected.data.provider, 'ariel-fixture');
  assert.equal(selected.data.id, 'fixture-a');
  const state = await rpc('get_state');
  assert.equal(state.success, true);
  assert.equal(state.data.model.id, 'fixture-a');
  assert.equal(state.data.model.provider, 'ariel-fixture');
  assert.equal(state.data.isStreaming, false);
  const invalid = await rpc('set_model', { provider: 'ariel-fixture', modelId: 'missing-model' });
  assert.equal(invalid.success, false);
  const unchanged = await rpc('get_state');
  assert.equal(unchanged.data.model.id, 'fixture-a');
  assert.equal((await rpc('abort')).success, true);
  child.stdin.end();
  const outcome = await waitExit(5000);
  assert.equal(outcome.code, 0);
  assert.equal(buffer.length, 0);
  if (fatal) throw fatal;
  const metadata = spawnSync(process.execPath, ['--input-type=module', '-e', `
    import { join } from 'node:path';
    const { ModelRuntime } = await import(process.argv[1]);
    const root = process.env.PI_CODING_AGENT_DIR;
    const runtime = await ModelRuntime.create({
      authPath: join(root, 'auth.json'), modelsPath: join(root, 'models.json'),
      modelsStorePath: join(root, 'models-store.json'),
      allowModelNetwork: false, refreshOnCreate: false,
    });
    console.log(JSON.stringify({ providers: runtime.getProviders().length,
      models: runtime.getModels().length,
      fixtureModels: runtime.getModels('ariel-fixture').map(model => model.id).sort() }));
  `, pathToFileURL(join(packageRoot, 'dist/core/model-runtime.js')).href], {
    cwd: root, env, encoding: 'utf8', timeout: 15000, maxBuffer: 1024 * 1024, windowsHide: true,
  });
  assert.equal(metadata.status, 0, 'Pi catalog metadata helper failed');
  const knownCatalog = JSON.parse(metadata.stdout);
  assert(knownCatalog.providers > 1);
  assert(knownCatalog.models > models.length);
  assert.deepEqual(knownCatalog.fixtureModels, ['fixture-a', 'fixture-b']);
  console.log(JSON.stringify({
    status: 'pass', package: manifest.name, version: manifest.version,
    cliSha256: createHash('sha256').update(readFileSync(cli)).digest('hex'),
    nodeVersion: process.version, platform: process.platform,
    checks: ['isolated-offline-start', 'catalog-two-synthetic-models', 'exact-model-selection',
      'state-confirmation', 'unknown-model-rejected-without-switch', 'idle-abort', 'stdin-eof-shutdown',
      'known-catalog-metadata-without-auth-or-network'],
    catalog: { knownProviders: knownCatalog.providers, knownModels: knownCatalog.models,
      rpcAvailableModels: catalog.data.models.length, syntheticModels: models.length },
    promptsSent: 0, realCredentialsSupplied: false, stderrBytes,
    limits: ['No model response, active abort, sandbox, tools or process-tree guarantee tested'],
  }, null, 2));
} finally {
  if (!closed) { child.kill(); await waitExit(5000); }
  const resolvedRoot = realpathSync(root);
  assert(resolvedRoot.startsWith(realpathSync(tmpdir()) + sep));
  assert(basename(resolvedRoot).startsWith('ariel-pi-rpc-'));
  assert.equal(resolve(resolvedRoot).toLowerCase(), resolve(root).toLowerCase());
  rmSync(resolvedRoot, { recursive: true });
}
