// node --test sample/tools/build-tools.test.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
	accessSync,
	chmodSync,
	constants,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	rmSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { after, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('../..', import.meta.url));
const SCRIPT = join(ROOT, 'sample/tools/build-tools.sh');
const PROGRAMS = ['m1a-extract', 'sifs-load', 'sifs-show'];
const DATA_FILES = ['banks.json', 'iban-registry.json'];
const PIECES = [...PROGRAMS, ...DATA_FILES];

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

const readFolder = (folder) => {
	const entries = readdirSync(folder).sort();
	const contents = {};
	for (const name of entries) {
		contents[name] = readFileSync(join(folder, name));
	}
	return contents;
};

const siblingsMatching = (target) =>
	readdirSync(dirname(target))
		.filter((name) => name.startsWith('.build-tools'))
		.sort();

const stderrLines = (res) => (res.stderr ? res.stderr.trimEnd().split('\n') : []);

test('a failed move while replacing an existing tools folder leaves the old set whole, never mixed', () => {
	const { dir, run } = makeCase();
	const out = join(dir, 'tools');
	const oldSet = writeOldSet(out);

	const res = run([out], { env: { FAKE_MV_FAIL_ON: '2' } });
	assert.equal(res.status, 1, `expected exit 1, got ${res.status}: ${res.stderr}`);
	assert.equal(stderrLines(res).length, 1, `expected one stderr line: ${res.stderr}`);
	assert.deepEqual(readFolder(out), oldSet);
	assert.deepEqual(siblingsMatching(out), []);
});
