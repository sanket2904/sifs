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
