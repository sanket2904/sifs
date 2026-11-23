// Runs a node test file in a child process and says how it ended. Only a
// child that exited by itself, failing, with every failure a named test of
// the file, says something about the code under test. Any other ending (a
// signal, no exit status, a file that did not load or whose process died, a
// failing status that names no test) is the runner's error, never a catch.
import { spawn } from 'node:child_process';
import { realpathSync } from 'node:fs';

// { status, signal, stdout } of node run with `args`. Started from inside a
// test run, a child node --test inherits NODE_TEST_CONTEXT and reports to that
// run instead of printing TAP, so the child gets an environment without it.
export function runNode(args) {
	const { NODE_TEST_CONTEXT, ...env } = process.env;
	return new Promise((done) => {
		const child = spawn(process.execPath, args, { env, stdio: ['ignore', 'pipe', 'inherit'] });
		let stdout = '';
		child.stdout.setEncoding('utf8').on('data', (chunk) => (stdout += chunk));
		child.on('error', () => done({ status: null, signal: null, stdout }));
		child.on('close', (status, signal) => done({ status, signal, stdout }));
	});
}

// passed, failed (with the names of the failing tests) or error (with why),
// from a run of the test file at `file` with the TAP reporter.
export function ending({ status, signal, stdout }, file) {
	const failed = [...stdout.matchAll(/^not ok \d+ - (.*)$/gm)].map((m) => m[1]);
	if (signal) return { kind: 'error', why: `the test process was ended by ${signal}` };
	if (!Number.isInteger(status)) return { kind: 'error', why: 'the test process gave no exit status' };
	if (status === 0) return { kind: 'passed' };
	if (failed.includes(file))
		return { kind: 'error', why: 'the test file did not load, or its process died' };
	if (!failed.length)
		return { kind: 'error', why: `the test process exited ${status} and named no failing test` };
	return { kind: 'failed', failed };
}

// Node names a file that fails as a whole by the path it was given, so the
// path is made real first and compared as given.
export async function runTestFile(file) {
	const path = realpathSync(file);
	return ending(await runNode(['--test', '--test-reporter=tap', path]), path);
}
