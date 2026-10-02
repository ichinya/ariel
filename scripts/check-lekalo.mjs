// Exercise the real Lekalo CLI against ARIEL and disposable malformed models.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const project = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const binary = process.env.LEKALO_BIN || 'lekalo';
const checks = [];
function run(args, cwd = project, expected = 0) {
  const result = spawnSync(binary, ['--no-cache', '--json', ...args], {
    cwd, encoding: 'utf8', timeout: 30000, maxBuffer: 16 * 1024 * 1024, windowsHide: true,
  });
  if (result.error) throw result.error;
  assert.equal(result.status, expected, `${args.join(' ')}: ${result.stderr || result.stdout}`);
  // Domain failures may use stderr even with --json; preserve the exit code.
  const output = result.stdout.trim() ? result.stdout : result.stderr;
  assert(output.trim(), `Missing JSON output: ${args.join(' ')}`);
  return { raw: output, value: JSON.parse(output) };
}
const model = run(['load', '--ir']);
assert.equal(model.value.status, 'valid');
assert(model.value.ir.definitions.some(item => item.id === 'app.submit_job'));
assert.equal(run(['load', '--ir']).raw, model.raw);
checks.push('canonical-ir-deterministic');
const validation = run(['validate']).value;
assert.equal(validation.validation.counts.error, 0);
assert.equal(validation.validation.counts.warning, 0);
checks.push('semantic-validation');
assert.equal(run(['lock', '--check', '--offline']).value.changed, false);
checks.push('lock-fresh');
const graph = run(['graph', 'show', 'app.submit_job']).value.graph;
assert(graph.dependencies.some(edge => edge.to === 'effect:app.submit_job_record'));
assert(graph.dependents.some(edge => edge.from === 'endpoint:app.jobs_http'));
checks.push('graph-command-effect-endpoint');
assert.equal(run(['inspect', 'app.model']).value.status, 'valid');
checks.push('inspect-model');
const capsule = run(['context', 'app.submit_job', '--budget', '8000']).value.context;
assert.equal(capsule.budget.fits, true);
assert(capsule.sections.scenarios.some(item => item.id === 'scenario:app.pi_job'));
checks.push('context-with-scenario');
const root = mkdtempSync(join(tmpdir(), 'ariel-lekalo-check-'));
try {
  cpSync(join(project, 'lekalo'), join(root, 'lekalo'), { recursive: true });
  const commandsPath = join(root, 'lekalo/modules/app/commands.yaml');
  const commands = JSON.parse(readFileSync(commandsPath, 'utf8'));
  commands.definitions.find(item => item.id === 'app.submit_job').effects = ['app.missing_effect'];
  writeFileSync(commandsPath, JSON.stringify(commands));
  const negative = run(['validate'], root, 1).value;
  assert.equal(negative.status, 'invalid');
  assert(negative.diagnostics.some(item => item.id === 'semantic.command-effect-unresolved'));
  checks.push('unresolved-effect-rejected');
  assert(!existsSync(join(root, '.lekalo')), '--no-cache unexpectedly wrote runtime state');
  checks.push('no-cache-leaves-fixture-clean');
} finally {
  const resolvedRoot = realpathSync(root);
  assert(resolvedRoot.startsWith(realpathSync(tmpdir()) + sep));
  assert(basename(resolvedRoot).startsWith('ariel-lekalo-check-'));
  assert.equal(resolve(resolvedRoot).toLowerCase(), resolve(root).toLowerCase());
  rmSync(resolvedRoot, { recursive: true });
}
const doctor = run(['doctor']).value;
assert.equal(doctor.revisions.lock.state, 'fresh');
assert(doctor.checks.filter(item => item.required).every(item => item.state === 'ok'));
checks.push('doctor-required-gates');
console.log(JSON.stringify({ status: 'pass', lekaloVersion: doctor.productVersion,
  modelDefinitionCount: model.value.ir.definitions.length,
  canonicalLoadSha256: createHash('sha256').update(model.raw).digest('hex'), checks,
  doctorVerdict: doctor.verdict,
  doctorFindings: doctor.checks.filter(item => item.state !== 'ok').map(item => ({ id: item.id, reason: item.reason })),
  contextGaps: capsule.gaps,
  limits: ['Model checks only; runtime implementation, permission enforcement and Rust code generation are not verified'],
}, null, 2));
