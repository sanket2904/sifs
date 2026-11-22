// PDFs written byte by byte, in the manner of
// src-tauri/src/extract/readers/macos/pdf/sample.rs. A line whose characters
// are all Latin-1 is shown in Helvetica, so it is visible. Any other line is
// shown in a font with no glyphs but a ToUnicode map: invisible, but its text
// is still read. An image page holds only a picture (see scan.mjs).
import { HEIGHT, WIDTH, render, runLength } from './scan.mjs';

const latin1 = (line) => [...line].every((c) => (c >= ' ' && c <= '~') || (c >= ' ' && c <= 'ÿ'));
const hex4 = (n) => n.toString(16).toUpperCase().padStart(4, '0');
const utf16hex = (s) =>
	[...s]
		.map((c) => [...Array(c.length).keys()].map((i) => hex4(c.charCodeAt(i))).join(''))
		.join('');

function literal(line) {
	let out = '';
	for (const c of line) {
		const code = c.charCodeAt(0);
		if (c === '(' || c === ')' || c === '\\') out += '\\' + c;
		else if (code > 0x7e) out += '\\' + code.toString(8).padStart(3, '0');
		else out += c;
	}
	return `(${out})`;
}

const pdfDate = (iso) => `D:${iso.replace(/[-:T]/g, '').replace('Z', '')}Z`;

export function pdf({ title, created, pages }) {
	const chars = [];
	for (const page of pages) {
		if (page.image) continue;
		for (const line of page.lines)
			if (!latin1(line)) for (const c of line) if (!chars.includes(c)) chars.push(c);
	}
	const objects = [];
	const add = (body) => objects.push(body);
	const stream = (data, dict = '') => {
		const bytes = typeof data === 'string' ? Buffer.from(data, 'latin1') : Buffer.from(data);
		return Buffer.concat([
			Buffer.from(`<< /Length ${bytes.length}${dict} >>\nstream\n`, 'latin1'),
			bytes,
			Buffer.from('\nendstream', 'latin1')
		]);
	};

	const catalog = add(null);
	const pagesObj = add(null);
	const info = add(
		`<< /Title <FEFF${utf16hex(title)}> /Producer (sample/tools/generate.mjs) /CreationDate (${pdfDate(created)}) /ModDate (${pdfDate(created)}) >>`
	);
	const helvetica = add(
		'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>'
	);
	let cmap =
		'/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n';
	for (let block = 0; block < chars.length; block += 100) {
		const group = chars.slice(block, block + 100);
		cmap += `${group.length} beginbfchar\n`;
		group.forEach((c, i) => (cmap += `<${hex4(block + i + 1)}> <${utf16hex(c)}>\n`));
		cmap += 'endbfchar\n';
	}
	cmap += 'endcmap CMapName currentdict /CMap defineresource pop end end';
	const toUnicode = add(stream(cmap));
	const descriptor = add(
		'<< /Type /FontDescriptor /FontName /T /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>'
	);
	const cidFont = add(
		`<< /Type /Font /Subtype /CIDFontType2 /BaseFont /T /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor ${descriptor} 0 R /DW 600 >>`
	);
	const unicodeFont = add(
		`<< /Type /Font /Subtype /Type0 /BaseFont /T /Encoding /Identity-H /DescendantFonts [${cidFont} 0 R] /ToUnicode ${toUnicode} 0 R >>`
	);

	const kids = [];
	for (const page of pages) {
		let content;
		let resources;
		if (page.image) {
			const image = add(
				stream(
					runLength(render(page.lines)),
					` /Type /XObject /Subtype /Image /Width ${WIDTH} /Height ${HEIGHT} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /RunLengthDecode`
				)
			);
			content = 'q 612 0 0 792 0 0 cm /Im0 Do Q';
			resources = `<< /XObject << /Im0 ${image} 0 R >> >>`;
		} else {
			if (page.lines.length > 52)
				throw new Error(`${page.lines.length} lines do not fit on a page`);
			content = page.lines
				.map((line, i) => {
					if (line === '') return null;
					const y = 760 - i * 14;
					const shown = latin1(line)
						? `/F2 10 Tf 50 ${y} Td ${literal(line)}`
						: `/F1 10 Tf 50 ${y} Td <${[...line].map((c) => hex4(chars.indexOf(c) + 1)).join('')}>`;
					return `BT ${shown} Tj ET`;
				})
				.filter(Boolean)
				.join('\n');
			resources = `<< /Font << /F1 ${unicodeFont} 0 R /F2 ${helvetica} 0 R >> >>`;
		}
		const contents = add(stream(content));
		kids.push(
			add(
				`<< /Type /Page /Parent ${pagesObj} 0 R /MediaBox [0 0 612 792] /Resources ${resources} /Contents ${contents} 0 R >>`
			)
		);
	}
	objects[catalog - 1] = `<< /Type /Catalog /Pages ${pagesObj} 0 R >>`;
	objects[pagesObj - 1] =
		`<< /Type /Pages /Kids [${kids.map((k) => `${k} 0 R`).join(' ')}] /Count ${kids.length} >>`;

	const parts = [Buffer.from('%PDF-1.7\n%\xe2\xe3\xcf\xd3\n', 'latin1')];
	let length = parts[0].length;
	const offsets = objects.map((body, i) => {
		const chunk = Buffer.concat([
			Buffer.from(`${i + 1} 0 obj\n`, 'latin1'),
			Buffer.isBuffer(body) ? body : Buffer.from(body, 'latin1'),
			Buffer.from('\nendobj\n', 'latin1')
		]);
		parts.push(chunk);
		const at = length;
		length += chunk.length;
		return at;
	});
	let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
	for (const offset of offsets) xref += `${String(offset).padStart(10, '0')} 00000 n \n`;
	xref += `trailer\n<< /Size ${objects.length + 1} /Root ${catalog} 0 R /Info ${info} 0 R >>\nstartxref\n${length}\n%%EOF\n`;
	parts.push(Buffer.from(xref, 'latin1'));
	return Buffer.concat(parts);
}
