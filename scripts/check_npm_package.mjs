#!/usr/bin/env node

import { spawnSync } from "node:child_process";

const requiredFiles = [
	// The Pi extension runs the installer when npm skipped the install script.
	"npm/install.js",
	"pi/extension/index.ts",
	"pi/extension/generated-tools.ts",
	"pi/extension/mcp-client.bundle.cjs",
	"pi/extension/THIRD_PARTY_NOTICES.txt",
];
// Two budgets: shipped code (the Pi bundle and catalog dominate) and the
// Markdown docs, which grow with every host recipe and should not push code
// over its limit or the other way round.
const maxCodeBytes = 650_000;
const maxDocsBytes = 200_000;
const isDoc = (path) => path.endsWith(".md");

const result = spawnSync(
	"npm",
	["pack", "--dry-run", "--json"],
	{ encoding: "utf8" },
);
if (result.status !== 0) {
	process.stderr.write(result.stderr);
	process.exit(result.status ?? 1);
}

const packed = JSON.parse(result.stdout);
const pack = Array.isArray(packed) ? packed[0] : Object.values(packed)[0];
const paths = new Set(pack.files.map((file) => file.path));
const missing = requiredFiles.filter((path) => !paths.has(path));
if (missing.length > 0) {
	throw new Error(`npm package is missing required files: ${missing.join(", ")}`);
}
const sum = (files) => files.reduce((total, file) => total + file.size, 0);
const docsBytes = sum(pack.files.filter((file) => isDoc(file.path)));
const codeBytes = sum(pack.files.filter((file) => !isDoc(file.path)));
const over = [];
if (codeBytes > maxCodeBytes) {
	over.push(`code ${codeBytes} exceeds ${maxCodeBytes} bytes`);
}
if (docsBytes > maxDocsBytes) {
	over.push(`docs ${docsBytes} exceed ${maxDocsBytes} bytes`);
}
if (over.length > 0) {
	throw new Error(`npm package unpacked size: ${over.join("; ")}`);
}

console.log(
	`npm package OK: ${pack.files.length} files, ${pack.size} bytes packed, ` +
		`${pack.unpackedSize} bytes unpacked (code ${codeBytes}, docs ${docsBytes})`,
);
