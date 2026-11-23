// node --test sample/tools/suite.test.mjs
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, test } from 'node:test';
import { ending, runNode, runTestFile } from './lib/suite.mjs';

const dir = mkdtempSync(join(tmpdir(), 'suite-test-'));
after(() => rmSync(dir, { recursive: true, force: true }));
const fixture = (name, source) => {
	const path = join(dir, `${name}.test.mjs`);
	writeFileSync(path, `import { test } from 'node:test';\n${source}\n`);
	return path;
};

test('a suite whose tests all pass has passed', async () => {
	assert.deepEqual(await runTestFile(fixture('pass', "test('fine', () => {});")), {
		kind: 'passed'
	});
});

test('a suite that exits by itself with named failing tests has failed, by those tests', async () => {
	const file = fixture(
		'fail',
		"test('broken', () => { throw new Error('no'); });\ntest('fine', () => {});"
	);
	assert.deepEqual(await runTestFile(file), { kind: 'failed', failed: ['broken'] });
});

test('a test file that does not load is the runner error, never a failing test', async () => {
	const file = fixture('load', "import './no-such-module.mjs';");
	assert.equal((await runTestFile(file)).kind, 'error');
});

test('a test process killed by a signal is the runner error', async () => {
	const file = fixture('killed', "test('killed', () => { process.kill(process.pid, 'SIGKILL'); });");
	assert.equal((await runTestFile(file)).kind, 'error');
});

test('a test process that exits by itself, failing, before any test is the runner error', async () => {
	assert.equal((await runTestFile(fixture('exits', 'process.exit(3);'))).kind, 'error');
});

test('a runner process ended by a signal is the runner error', async () => {
	const run = await runNode(['-e', "process.kill(process.pid, 'SIGKILL')"]);
	assert.equal(run.signal, 'SIGKILL');
	assert.deepEqual(ending(run, 'x.test.mjs'), {
		kind: 'error',
		why: 'the test process was ended by SIGKILL'
	});
});

test('a failing status that names no test is the runner error', async () => {
	const run = await runNode(['-e', "console.log('not ok, but no test'); process.exit(1)"]);
	assert.equal(run.status, 1);
	assert.equal(ending(run, 'x.test.mjs').kind, 'error');
});

test('a run with no exit status is the runner error', () => {
	assert.equal(ending({ status: null, signal: null, stdout: 'not ok 1 - broken\n' }, 'x').kind, 'error');
});
