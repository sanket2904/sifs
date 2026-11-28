// node --test sample/tools/build-tools.test.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
	accessSync,
	chmodSync,
	constants,
	existsSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	rmSync,
	statSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import net from 'node:net';

const ROOT = fileURLToPath(new URL('../..', import.meta.url));
const SCRIPT = join(ROOT, 'sample/tools/build-tools.sh');
const PROGRAMS = ['m1a-extract', 'sifs-load', 'sifs-show'];
const DATA_FILES = ['banks.json', 'iban-registry.json'];
const COMMITTED_DATA = {
	'banks.json': readFileSync(join(ROOT, 'sample/directory/banks.json')),
	'iban-registry.json': readFileSync(join(ROOT, 'sample/iban-registry.json'))
};

const findBin = (name) => {
	for (const dir of (process.env.PATH ?? '').split(':')) {
		if (!dir) continue;
		const candidate = join(dir, name);
		try {
			accessSync(candidate, constants.X_OK);
			return candidate;
		} catch {}
	}
	throw new Error(`cannot find ${name} on PATH`);
};

const REAL_MV = findBin('mv');
const REAL_RM = findBin('rm');

const work = mkdtempSync(join(tmpdir(), 'build-tools-test-'));
after(() => rmSync(work, { recursive: true, force: true }));

let caseId = 0;
const makeCase = () => {
	const dir = join(work, `case-${++caseId}`);
	const bin = join(dir, 'bin');
	mkdirSync(bin, { recursive: true });
	const goLog = join(dir, 'go.log');
	const mvCount = join(dir, 'mv.count');

	const writeExec = (path, body) => {
		writeFileSync(path, body);
		chmodSync(path, 0o755);
	};

	writeExec(
		join(bin, 'go'),
		`#!/bin/sh
printf '%s\\n' "$*" >> "${goLog}"
[ "$GOWORK" = off ] || { echo "expected GOWORK=off" >&2; exit 99; }
out=""
while [ $# -gt 0 ]; do
	case "$1" in
		-o) out="$2"; shift ;;
		./cmd/*)
			name="\${1##*/}"
			if [ "$name" != "m1a-extract" ] && [ "\${GOPRIVATE:-}" != "github.com/sanket2904/ontology-framework" ]; then
				echo "expected GOPRIVATE for $name" >&2
				exit 98
			fi
			if [ "\${FAKE_GO_FAIL:-}" = "$name" ]; then
				echo "simulated build failure for $name" >&2
				exit 1
			fi
			printf '#!/bin/sh\\n# %s %s\\n' "\${FAKE_GO_TAG:-new}" "$name" > "$out/$name"
			chmod 755 "$out/$name"
			;;
	esac
	shift
done
`
	);

	writeExec(
		join(bin, 'mv'),
		`#!/bin/sh
count=0
[ -f "${mvCount}" ] && count=$(cat "${mvCount}")
count=$((count + 1))
printf '%s\\n' "$count" > "${mvCount}"
if [ -n "\${FAKE_MV_FAIL_ON:-}" ] && [ "$count" = "$FAKE_MV_FAIL_ON" ]; then
	echo "mv: simulated failure on call $count" >&2
	exit 1
fi
exec "${REAL_MV}" "$@"
`
	);

	writeExec(
		join(bin, 'rm'),
		`#!/bin/sh
if [ -n "\${FAKE_RM_FAIL_AFTER_DIR:-}" ] && [ -f "$FAKE_RM_FAIL_AFTER_DIR/m1a-extract" ] && grep -q "^# \${FAKE_GO_TAG:-new} " "$FAKE_RM_FAIL_AFTER_DIR/m1a-extract" 2>/dev/null; then
	echo "rm: simulated removal failure" >&2
	exit 1
fi
exec "${REAL_RM}" "$@"
`
	);

	const run = (args, opts = {}) =>
		spawnSync(SCRIPT, args, {
			cwd: opts.cwd ?? ROOT,
			encoding: 'utf8',
			env: {
				...process.env,
				PATH: `${bin}:${process.env.PATH ?? ''}`,
				...opts.env
			}
		});

	return { dir, goLog, run };
};

const expectedNewSet = (tag = 'new') => ({
	'm1a-extract': Buffer.from(`#!/bin/sh\n# ${tag} m1a-extract\n`),
	'sifs-load': Buffer.from(`#!/bin/sh\n# ${tag} sifs-load\n`),
	'sifs-show': Buffer.from(`#!/bin/sh\n# ${tag} sifs-show\n`),
	...COMMITTED_DATA
});

const writeOldSet = (folder) => {
	mkdirSync(folder, { recursive: true });
	const expected = {};
	for (const name of PROGRAMS) {
		const content = Buffer.from(`#!/bin/sh\n# old ${name}\n`);
		writeFileSync(join(folder, name), content);
		chmodSync(join(folder, name), 0o755);
		expected[name] = content;
	}
	for (const name of DATA_FILES) {
		const content = Buffer.from(`old ${name}\n`);
		writeFileSync(join(folder, name), content);
		expected[name] = content;
	}
	return expected;
};

const readTree = (folder) => {
	const out = {};
	for (const name of readdirSync(folder).sort()) {
		const path = join(folder, name);
		const st = lstatSync(path);
		if (st.isDirectory()) {
			out[name] = { dir: readTree(path) };
		} else if (st.isSymbolicLink()) {
			out[name] = { symlink: true };
		} else {
			out[name] = readFileSync(path);
		}
	}
	return out;
};

const assertTreeEqual = (actual, expected, prefix = '') => {
	assert.deepEqual(Object.keys(actual).sort(), Object.keys(expected).sort(), `${prefix}entries differ`);
	for (const name of Object.keys(expected).sort()) {
		const a = actual[name];
		const e = expected[name];
		const label = prefix ? `${prefix}/${name}` : name;
		if (Buffer.isBuffer(a) && Buffer.isBuffer(e)) {
			if (!a.equals(e)) {
				assert.fail(
					`${label} differs (actual ${a.length} bytes: ${JSON.stringify(a.subarray(0, 80).toString('utf8'))}, expected ${e.length} bytes: ${JSON.stringify(e.subarray(0, 80).toString('utf8'))})`
				);
			}
		} else if (a?.dir && e?.dir) {
			assertTreeEqual(a.dir, e.dir, label);
		} else {
			assert.deepEqual(a, e, `${label} differs`);
		}
	}
};

const assertRunnablePrograms = (folder) => {
	for (const name of PROGRAMS) {
		const mode = statSync(join(folder, name)).mode;
		assert.notEqual(mode & 0o111, 0, `${name} should be runnable`);
	}
};

const siblingsMatching = (target) =>
	readdirSync(dirname(target))
		.filter((name) => name.startsWith('.build-tools'))
		.sort();

const stderrLines = (res) => (res.stderr ? res.stderr.trimEnd().split('\n') : []);

test('no argument, empty argument, or extra arguments print usage and exit 2', () => {
	for (const args of [[], [''], ['a', 'b']]) {
		const { goLog, run } = makeCase();
		const res = run(args);
		assert.equal(res.status, 2, `args ${JSON.stringify(args)}: ${res.stderr}`);
		assert.equal(stderrLines(res).length, 1);
		assert.match(res.stderr, /^usage: /);
		assert.equal(existsSync(goLog), false, 'go was not called');
	}
});

test('a fresh folder gets all five pieces with runnable programs and no leftovers', () => {
	const { dir, run } = makeCase();
	const out = join(dir, 'fresh', 'tools');
	const expected = expectedNewSet();

	const res = run([out]);
	assert.equal(res.status, 0, res.stderr);
	assertTreeEqual(readTree(out), expected);
	assertRunnablePrograms(out);
	assert.deepEqual(siblingsMatching(out), []);
});

test('an existing older full set is replaced with all five new pieces', () => {
	const { dir, run } = makeCase();
	const out = join(dir, 'tools');
	writeOldSet(out);
	const expected = expectedNewSet();

	const res = run([out]);
	assert.equal(res.status, 0, res.stderr);
	assertTreeEqual(readTree(out), expected);
	assertRunnablePrograms(out);
	assert.deepEqual(siblingsMatching(out), []);
});

for (const fail of ['m1a-extract', 'sifs-load']) {
	test(`when building ${fail} fails on a fresh folder, exit 1 with one line and leave no folder`, () => {
		const { dir, run } = makeCase();
		const out = join(dir, 'tools');

		const res = run([out], { env: { FAKE_GO_FAIL: fail } });
		assert.equal(res.status, 1);
		assert.equal(stderrLines(res).length, 1, res.stderr);
		assert.equal(existsSync(out), false);
		assert.deepEqual(siblingsMatching(out), []);
	});

	test(`when building ${fail} fails on an existing folder, exit 1 with one line and leave the old set`, () => {
		const { dir, run } = makeCase();
		const out = join(dir, 'tools');
		const oldSet = writeOldSet(out);

		const res = run([out], { env: { FAKE_GO_FAIL: fail } });
		assert.equal(res.status, 1);
		assert.equal(stderrLines(res).length, 1, res.stderr);
		assertTreeEqual(readTree(out), oldSet);
		assert.deepEqual(siblingsMatching(out), []);
	});
}

test('a failed move into a fresh folder leaves no folder and no leftovers', () => {
	const { dir, run } = makeCase();
	const out = join(dir, 'tools');

	const res = run([out], { env: { FAKE_MV_FAIL_ON: '1' } });
	assert.equal(res.status, 1);
	assert.equal(stderrLines(res).length, 1, res.stderr);
	assert.equal(existsSync(out), false);
	assert.deepEqual(siblingsMatching(out), []);
});

test('failing to move the old folder aside leaves the old set whole and no leftovers', () => {
	const { dir, run } = makeCase();
	const out = join(dir, 'tools');
	const oldSet = writeOldSet(out);

	const res = run([out], { env: { FAKE_MV_FAIL_ON: '1' } });
	assert.equal(res.status, 1);
	assert.equal(stderrLines(res).length, 1, res.stderr);
	assertTreeEqual(readTree(out), oldSet);
	assert.deepEqual(siblingsMatching(out), []);
});

test('a failed move while replacing an existing tools folder restores the old set whole, never mixed', () => {
	const { dir, run } = makeCase();
	const out = join(dir, 'tools');
	const oldSet = writeOldSet(out);

	const res = run([out], { env: { FAKE_MV_FAIL_ON: '2' } });
	assert.equal(res.status, 1, `expected exit 1, got ${res.status}: ${res.stderr}`);
	assert.equal(stderrLines(res).length, 1, `expected one stderr line: ${res.stderr}`);
	assertTreeEqual(readTree(out), oldSet);
	assert.deepEqual(siblingsMatching(out), []);
});

test('when removing the old folder fails after publishing the new one, the new set is in place and the single leftover is named', () => {
	const { dir, run } = makeCase();
	const out = join(dir, 'tools');
	const oldSet = writeOldSet(out);
	const expected = expectedNewSet();

	const res = run([out], { env: { FAKE_RM_FAIL_AFTER_DIR: out } });
	assert.equal(res.status, 1);
	assert.equal(stderrLines(res).length, 1, res.stderr);
	assertTreeEqual(readTree(out), expected);
	assertRunnablePrograms(out);

	const leftovers = siblingsMatching(out);
	assert.equal(leftovers.length, 1, `expected one leftover old folder, got ${JSON.stringify(leftovers)}`);
	const leftoverPath = join(dirname(out), leftovers[0]);
	assert.ok(res.stderr.includes(leftoverPath), `stderr should name ${leftoverPath}: ${res.stderr}`);
	assertTreeEqual(readTree(leftoverPath), oldSet);
});

for (const [label, populate] of [
	['one extra file', (out) => writeFileSync(join(out, 'notes.txt'), 'keep me\n')],
	['one hidden file', (out) => writeFileSync(join(out, '.keep'), 'hidden\n')],
	['a sub-folder', (out) => mkdirSync(join(out, 'subdir'))],
	[
		'a piece name that is a sub-folder',
		(out) => {
			rmSync(join(out, 'banks.json'));
			mkdirSync(join(out, 'banks.json'));
		}
	]
]) {
	test(`an existing folder holding ${label} is refused with exit 2 before building`, () => {
		const { dir, goLog, run } = makeCase();
		const out = join(dir, 'tools');
		writeOldSet(out);
		populate(out);
		const before = readTree(out);

		const res = run([out]);
		assert.equal(res.status, 2, res.stderr);
		assert.equal(stderrLines(res).length, 1, res.stderr);
		assert.ok(res.stderr.includes(out), `stderr should name ${out}: ${res.stderr}`);
		assert.match(res.stderr, /folder of their own/);
		assert.equal(existsSync(goLog), false, 'go was never called');
		assertTreeEqual(readTree(out), before);
		assert.deepEqual(siblingsMatching(out), []);
	});
}

test('an existing path that is a file or symlink is refused with exit 2 before building', () => {
	{
		const { dir, goLog, run } = makeCase();
		const out = join(dir, 'not-a-folder');
		const content = Buffer.from('important data\n');
		writeFileSync(out, content);

		const res = run([out]);
		assert.equal(res.status, 2, res.stderr);
		assert.equal(stderrLines(res).length, 1, res.stderr);
		assert.ok(res.stderr.includes(out), `stderr should name ${out}: ${res.stderr}`);
		assert.match(res.stderr, /folder of their own/);
		assert.equal(existsSync(goLog), false, 'go was never called');
		assert.deepEqual(readFileSync(out), content);
		assert.deepEqual(siblingsMatching(out), []);
	}
	{
		const { dir, goLog, run } = makeCase();
		const target = join(dir, 'real-dir');
		writeOldSet(target);
		const out = join(dir, 'link-to-dir');
		symlinkSync(target, out);

		const res = run([out]);
		assert.equal(res.status, 2, res.stderr);
		assert.equal(stderrLines(res).length, 1, res.stderr);
		assert.ok(res.stderr.includes(out), `stderr should name ${out}: ${res.stderr}`);
		assert.equal(existsSync(goLog), false, 'go was never called');
		assert.equal(lstatSync(out).isSymbolicLink(), true);
		assert.deepEqual(siblingsMatching(out), []);
	}
});

test('folder paths with a space and with a leading dash work for fresh and existing folders', () => {
	const { dir, run } = makeCase();
	const expected = expectedNewSet();

	const spaced = join(dir, 'folder with spaces', 'my tools');
	let res = run([spaced]);
	assert.equal(res.status, 0, res.stderr);
	assertTreeEqual(readTree(spaced), expected);
	assertRunnablePrograms(spaced);
	assert.deepEqual(siblingsMatching(spaced), []);

	const dashed = join(dir, '-tools');
	res = run(['-tools'], { cwd: dir });
	assert.equal(res.status, 0, res.stderr);
	assertTreeEqual(readTree(dashed), expected);
	assertRunnablePrograms(dashed);
	assert.deepEqual(siblingsMatching(dashed), []);

	const updated = expectedNewSet('v2');
	res = run(['-tools'], { cwd: dir, env: { FAKE_GO_TAG: 'v2' } });
	assert.equal(res.status, 0, res.stderr);
	assertTreeEqual(readTree(dashed), updated);
	assertRunnablePrograms(dashed);
	assert.deepEqual(siblingsMatching(dashed), []);
});

test('running the script twice in a row produces the same result and leaves nothing behind', () => {
	const { dir, run } = makeCase();
	const out = join(dir, 'tools');
	const expected = expectedNewSet();

	const first = run([out]);
	assert.equal(first.status, 0, first.stderr);
	assertTreeEqual(readTree(out), expected);
	assert.deepEqual(siblingsMatching(out), []);

	const second = run([out]);
	assert.equal(second.status, 0, second.stderr);
	assertTreeEqual(readTree(out), expected);
	assertRunnablePrograms(out);
	assert.deepEqual(siblingsMatching(out), []);
});

test('running the script from another working folder works for relative and absolute targets', () => {
	const { dir, run } = makeCase();
	const otherCwd = join(dir, 'elsewhere');
	mkdirSync(otherCwd, { recursive: true });
	const expected = expectedNewSet();

	const relRes = run(['rel-tools'], { cwd: otherCwd });
	assert.equal(relRes.status, 0, relRes.stderr);
	const relOut = join(otherCwd, 'rel-tools');
	assertTreeEqual(readTree(relOut), expected);
	assertRunnablePrograms(relOut);
	assert.deepEqual(siblingsMatching(relOut), []);

	const absOut = join(dir, 'abs-tools');
	const absRes = run([absOut], { cwd: otherCwd });
	assert.equal(absRes.status, 0, absRes.stderr);
	assertTreeEqual(readTree(absOut), expected);
	assertRunnablePrograms(absOut);
	assert.deepEqual(siblingsMatching(absOut), []);
});

const GATE_HELPER_SRC = `import { spawnSync } from 'node:child_process';
import net from 'node:net';

const [port, realBin, ...args] = process.argv.slice(2);
const timer = setTimeout(() => process.exit(124), 20_000);
const sock = net.connect(Number(port), '127.0.0.1');
let buf = '';
let done = false;
const finish = (code) => {
	if (done) return;
	done = true;
	clearTimeout(timer);
	sock.destroy();
	process.exit(code);
};
sock.on('error', () => finish(125));
sock.on('data', (chunk) => {
	buf += chunk.toString('utf8');
	const nl = buf.indexOf('\\n');
	if (nl === -1) return;
	const action = buf.slice(0, nl).trim();
	if (action === 'do') {
		const res = spawnSync(realBin, args, { stdio: 'inherit' });
		finish(res.status ?? 1);
	} else {
		finish(Number(action) || 1);
	}
});
sock.on('end', () => {
	if (!done) finish(125);
});
`;

const SIGNAL_EXIT_CODE = { SIGHUP: 129, SIGINT: 130, SIGTERM: 143 };

const makeInterruptCase = () => {
	const dir = join(work, `case-${++caseId}`);
	const bin = join(dir, 'bin');
	mkdirSync(bin, { recursive: true });
	const mvCount = join(dir, 'mv.count');
	const rmOldCount = join(dir, 'rm-old.count');
	const goCount = join(dir, 'go.count');
	const gateScript = join(dir, 'gate.mjs');
	const goReal = join(dir, 'go-real');
	writeFileSync(gateScript, GATE_HELPER_SRC);

	const writeExec = (path, body) => {
		writeFileSync(path, body);
		chmodSync(path, 0o755);
	};

	writeExec(
		goReal,
		`#!/bin/sh
[ "$GOWORK" = off ] || { echo "expected GOWORK=off" >&2; exit 99; }
out=""
while [ $# -gt 0 ]; do
	case "$1" in
		-o) out="$2"; shift ;;
		./cmd/*)
			name="\${1##*/}"
			if [ "$name" != "m1a-extract" ] && [ "\${GOPRIVATE:-}" != "github.com/sanket2904/ontology-framework" ]; then
				echo "expected GOPRIVATE for $name" >&2
				exit 98
			fi
			printf '#!/bin/sh\\n# %s %s\\n' "\${FAKE_GO_TAG:-new}" "$name" > "$out/$name"
			chmod 755 "$out/$name"
			;;
	esac
	shift
done
`
	);

	writeExec(
		join(bin, 'go'),
		`#!/bin/sh
count=0
[ -f "${goCount}" ] && count=$(cat "${goCount}")
count=$((count + 1))
printf '%s\\n' "$count" > "${goCount}"
if [ -n "\${FAKE_GO_GATE_ON:-}" ] && [ "$count" = "$FAKE_GO_GATE_ON" ]; then
	exec "${process.execPath}" "${gateScript}" "$FAKE_GATE_PORT" "${goReal}" "$@"
fi
exec "${goReal}" "$@"
`
	);

	writeExec(
		join(bin, 'mv'),
		`#!/bin/sh
count=0
[ -f "${mvCount}" ] && count=$(cat "${mvCount}")
count=$((count + 1))
printf '%s\\n' "$count" > "${mvCount}"
if [ -n "\${FAKE_MV_FAIL_ON:-}" ] && [ "$count" = "$FAKE_MV_FAIL_ON" ]; then
	echo "mv: simulated failure on call $count" >&2
	exit 1
fi
if [ -n "\${FAKE_MV_GATE_ON:-}" ] && [ "$count" = "$FAKE_MV_GATE_ON" ]; then
	exec "${process.execPath}" "${gateScript}" "$FAKE_GATE_PORT" "${REAL_MV}" "$@"
fi
exec "${REAL_MV}" "$@"
`
	);

	writeExec(
		join(bin, 'rm'),
		`#!/bin/sh
if [ -n "\${FAKE_RM_GATE_OLD:-}" ]; then
	for arg in "$@"; do
		case "$arg" in
			*.old)
				count=0
				[ -f "${rmOldCount}" ] && count=$(cat "${rmOldCount}")
				count=$((count + 1))
				printf '%s\\n' "$count" > "${rmOldCount}"
				if [ "$count" = "1" ]; then
					exec "${process.execPath}" "${gateScript}" "$FAKE_GATE_PORT" "${REAL_RM}" "$@"
				fi
				;;
		esac
	done
fi
exec "${REAL_RM}" "$@"
`
	);

	const runGated = (args, { signal, standinAction, env = {} }) =>
		new Promise((resolve, reject) => {
			let child;
			let gateHit = false;
			let resolveStandin;
			const standinDone = new Promise((r) => {
				resolveStandin = r;
			});

			const srv = net.createServer((conn) => {
				gateHit = true;
				conn.on('close', () => resolveStandin());
				process.kill(child.pid, signal);
				const cmd = standinAction === 'do' ? 'do' : String(SIGNAL_EXIT_CODE[signal]);
				conn.end(`${cmd}\n`);
			});
			srv.on('error', reject);
			srv.listen(0, '127.0.0.1', () => {
				const { port } = srv.address();
				child = spawn(SCRIPT, args, {
					cwd: ROOT,
					env: {
						...process.env,
						PATH: `${bin}:${process.env.PATH ?? ''}`,
						FAKE_GATE_PORT: String(port),
						...env
					}
				});
				let stdout = '';
				let stderr = '';
				child.stdout.setEncoding('utf8');
				child.stderr.setEncoding('utf8');
				child.stdout.on('data', (d) => {
					stdout += d;
				});
				child.stderr.on('data', (d) => {
					stderr += d;
				});
				child.on('error', (err) => {
					srv.close();
					reject(err);
				});
				child.on('close', async (status, exitSignal) => {
					if (!gateHit) resolveStandin();
					await standinDone;
					srv.close();
					resolve({ status, signal: exitSignal, stdout, stderr, gateHit });
				});
			});
		});

	const runPlain = (args, env = {}) =>
		spawnSync(SCRIPT, args, {
			cwd: ROOT,
			encoding: 'utf8',
			env: {
				...process.env,
				PATH: `${bin}:${process.env.PATH ?? ''}`,
				...env
			}
		});

	return { dir, runGated, runPlain };
};

const assertInterruptedState = (res, out, expectedState, oldSet) => {
	assert.equal(res.gateHit, true, 'gate was hit');
	assert.ok(
		(res.status !== null && res.status !== 0) || res.signal !== null,
		`expected non-zero exit, got status=${res.status} signal=${res.signal}`
	);
	const siblings = siblingsMatching(out);
	const lines = stderrLines(res);
	if (expectedState === 'old') {
		assert.equal(
			existsSync(out),
			true,
			`target ${out} missing after interrupt (siblings=${JSON.stringify(siblings)}, stderr=${JSON.stringify(res.stderr)})`
		);
		assertTreeEqual(readTree(out), oldSet);
		assert.deepEqual(siblings, [], `leftovers beside ${out}: ${JSON.stringify(siblings)}`);
		assert.equal(lines.length, 1, `expected one stderr line, got ${JSON.stringify(lines)}`);
		assert.ok(lines[0].includes(out), `stderr should name ${out}: ${res.stderr}`);
		assert.match(lines[0], /previous|old/i, `stderr should say old/previous set: ${res.stderr}`);
		assert.doesNotMatch(lines[0], /\bnew\b/i, `stderr should not say new set: ${res.stderr}`);
	} else if (expectedState === 'new') {
		assert.equal(
			existsSync(out),
			true,
			`target ${out} missing after interrupt (siblings=${JSON.stringify(siblings)}, stderr=${JSON.stringify(res.stderr)})`
		);
		assertTreeEqual(readTree(out), expectedNewSet());
		assertRunnablePrograms(out);
		assert.deepEqual(siblings, [], `leftovers beside ${out}: ${JSON.stringify(siblings)}`);
		assert.equal(lines.length, 1, `expected one stderr line, got ${JSON.stringify(lines)}`);
		assert.ok(lines[0].includes(out), `stderr should name ${out}: ${res.stderr}`);
		assert.match(lines[0], /\bnew\b/i, `stderr should say new set: ${res.stderr}`);
		assert.doesNotMatch(lines[0], /previous|old/i, `stderr should not say old set: ${res.stderr}`);
	} else {
		assert.equal(existsSync(out), false, `target ${out} should remain absent`);
		assert.deepEqual(siblings, [], `leftovers beside ${out}: ${JSON.stringify(siblings)}`);
		assert.equal(lines.length, 1, `expected one stderr line, got ${JSON.stringify(lines)}`);
		assert.ok(lines[0].includes(out), `stderr should name ${out}: ${res.stderr}`);
		assert.match(lines[0], /not created|absent/i, `stderr should say absent/not created: ${res.stderr}`);
		assert.doesNotMatch(lines[0], /\bnew\b/i, `stderr should not say new set: ${res.stderr}`);
	}
};

const assertRerunWorks = (runPlain, out) => {
	const rerun = runPlain([out]);
	assert.equal(rerun.status, 0, `rerun failed: ${rerun.stderr}`);
	assertTreeEqual(readTree(out), expectedNewSet());
	assertRunnablePrograms(out);
	assert.deepEqual(siblingsMatching(out), []);
};

for (const step of [
	{ name: 'first rename', env: { FAKE_MV_GATE_ON: '1' }, didState: 'old', abortedState: 'old' },
	{ name: 'second rename', env: { FAKE_MV_GATE_ON: '2' }, didState: 'new', abortedState: 'old' },
	{ name: 'old folder removal', env: { FAKE_RM_GATE_OLD: '1' }, didState: 'new', abortedState: 'new' }
]) {
	for (const signal of ['SIGINT', 'SIGTERM']) {
		for (const [standinAction, expectedState] of [
			['do', step.didState],
			['abort', step.abortedState]
		]) {
			test(`interrupt (${signal}) during ${step.name} (${standinAction}) leaves complete ${expectedState} set and no leftovers`, async () => {
				const { dir, runGated, runPlain } = makeInterruptCase();
				const out = join(dir, 'tools');
				const oldSet = writeOldSet(out);

				const res = await runGated([out], { signal, standinAction, env: step.env });
				assertInterruptedState(res, out, expectedState, oldSet);
				assertRerunWorks(runPlain, out);
			});
		}
	}
}

test('interrupt (SIGHUP) during first rename (do) restores the old set and leaves no leftovers', async () => {
	const { dir, runGated, runPlain } = makeInterruptCase();
	const out = join(dir, 'tools');
	const oldSet = writeOldSet(out);

	const res = await runGated([out], { signal: 'SIGHUP', standinAction: 'do', env: { FAKE_MV_GATE_ON: '1' } });
	assertInterruptedState(res, out, 'old', oldSet);
	assertRerunWorks(runPlain, out);
});

test('interrupt (SIGINT) during build leaves existing target untouched and no leftovers', async () => {
	const { dir, runGated, runPlain } = makeInterruptCase();
	const out = join(dir, 'tools');
	const oldSet = writeOldSet(out);

	const res = await runGated([out], { signal: 'SIGINT', standinAction: 'abort', env: { FAKE_GO_GATE_ON: '1' } });
	assertInterruptedState(res, out, 'old', oldSet);
	assertRerunWorks(runPlain, out);
});

test('interrupt on a fresh target leaves it absent when rename aborts and complete when rename finishes', async () => {
	{
		const { dir, runGated, runPlain } = makeInterruptCase();
		const out = join(dir, 'tools');

		const res = await runGated([out], { signal: 'SIGINT', standinAction: 'abort', env: { FAKE_MV_GATE_ON: '1' } });
		assertInterruptedState(res, out, 'absent', null);
		assertRerunWorks(runPlain, out);
	}
	{
		const { dir, runGated, runPlain } = makeInterruptCase();
		const out = join(dir, 'tools');

		const res = await runGated([out], { signal: 'SIGTERM', standinAction: 'do', env: { FAKE_MV_GATE_ON: '1' } });
		assertInterruptedState(res, out, 'new', null);
		assertRerunWorks(runPlain, out);
	}
});

test('interrupt during first rename when restoring old set fails names the hidden old folder', async () => {
	const { dir, runGated } = makeInterruptCase();
	const out = join(dir, 'tools');
	const oldSet = writeOldSet(out);

	const res = await runGated([out], {
		signal: 'SIGTERM',
		standinAction: 'do',
		env: { FAKE_MV_GATE_ON: '1', FAKE_MV_FAIL_ON: '2' }
	});
	assert.equal(res.gateHit, true, 'gate was hit');
	assert.ok((res.status !== null && res.status !== 0) || res.signal !== null);
	assert.equal(existsSync(out), false, `target ${out} should be absent when restore failed`);
	const leftovers = siblingsMatching(out);
	assert.equal(leftovers.length, 1, `expected only the hidden old folder, got ${JSON.stringify(leftovers)}`);
	const prevPath = join(dirname(out), leftovers[0]);
	assertTreeEqual(readTree(prevPath), oldSet);
	const lines = stderrLines(res);
	assert.equal(lines.length, 1, `expected one stderr line, got ${JSON.stringify(lines)}`);
	assert.ok(lines[0].includes(prevPath), `stderr should name ${prevPath}: ${res.stderr}`);
});
