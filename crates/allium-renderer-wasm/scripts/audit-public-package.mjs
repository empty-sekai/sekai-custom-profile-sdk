import { execFileSync } from "node:child_process";
import { readdir, readFile, stat } from "node:fs/promises";
import { basename, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const repositoryRoot = resolve(packageRoot, "../..");
const textExtensions = new Set([".css", ".html", ".js", ".json", ".md", ".mjs", ".rs", ".sh", ".toml", ".ts"]);
const forbiddenBinaryExtensions = new Set([".gif", ".jpeg", ".jpg", ".otf", ".png", ".ttc", ".ttf", ".webp", ".woff", ".woff2"]);
const forbiddenText = [
  { label: "CJK public text", pattern: /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u },
  { label: "Windows private workspace path", pattern: /[A-Z]:\\allium\\/i },
  { label: "internal shipping source", pattern: /\/shipping\//i },
  {
    label: "private repository reference",
    pattern: new RegExp([["allium", "scapus"].join("-"), ["sdf", "card", "browser", "next"].join("-")].join("|"), "i"),
  },
];

const authoredRoots = [
  join(repositoryRoot, ".gitignore"),
  join(repositoryRoot, "README.md"),
  join(repositoryRoot, "README.en.md"),
  join(packageRoot, "README.md"),
  join(packageRoot, "README.en.md"),
  join(packageRoot, "demo"),
  join(packageRoot, "scripts"),
  join(packageRoot, "src"),
  join(repositoryRoot, "docs"),
];

const failures = [];
for (const root of authoredRoots) {
  for (const path of await walk(root)) {
    if (!textExtensions.has(extname(path).toLowerCase())) continue;
    const content = await readFile(path, "utf8");
    for (const rule of forbiddenText) {
      if (rule.label === "CJK public text" && /^README(?:\.[^.]+)?\.md$/i.test(basename(path))) continue;
      if (rule.pattern.test(content)) failures.push(`${relative(repositoryRoot, path)}: ${rule.label}`);
    }
  }
}

const pack = JSON.parse(execFileSync("npm", ["pack", "--dry-run", "--json"], {
  cwd: packageRoot,
  encoding: "utf8",
}));
const files = pack[0]?.files?.map((entry) => entry.path) ?? [];
for (const path of files) {
  if (forbiddenBinaryExtensions.has(extname(path).toLowerCase())) {
    failures.push(`${path}: bundled font or game-image candidate`);
  }
  if (/(^|\/)(fixtures?|masterdata|profiles?|game-assets?)(\/|$)/i.test(path)) {
    failures.push(`${path}: bundled data/fixture directory`);
  }
}

if (failures.length > 0) {
  throw new Error(`Public package audit failed:\n${[...new Set(failures)].sort().join("\n")}`);
}

console.log(JSON.stringify({ authoredFiles: (await Promise.all(authoredRoots.map(walk))).flat().length, packedFiles: files.length }));

async function walk(path) {
  const metadata = await stat(path);
  if (metadata.isFile()) return [path];
  const output = [];
  for (const entry of await readdir(path, { withFileTypes: true })) {
    if (["dist", "node_modules", "target"].includes(entry.name)) continue;
    const child = join(path, entry.name);
    if (entry.isDirectory()) output.push(...await walk(child));
    else if (entry.isFile()) output.push(child);
  }
  return output;
}
