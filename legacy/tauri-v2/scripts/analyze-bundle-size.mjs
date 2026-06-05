import { gzipSync } from "node:zlib";
import { mkdir, readdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";

const root = process.cwd();
const distDir = path.join(root, "dist", "assets");
const outputDir = path.join(root, "test-results", "perf");
const outputPath = path.join(outputDir, "bundle-size.json");

const files = await readdir(distDir).catch((error) => {
  throw new Error(`dist/assets is unavailable; run pnpm build first. ${error.message}`);
});

const assets = [];
for (const filename of files) {
  const filePath = path.join(distDir, filename);
  const metadata = await stat(filePath);
  if (!metadata.isFile()) {
    continue;
  }

  const bytes = await readFile(filePath);
  assets.push({
    bytes: metadata.size,
    filename,
    gzipBytes: gzipSync(bytes).length,
    kind: filename.endsWith(".js")
      ? "javascript"
      : filename.endsWith(".css")
        ? "css"
        : "other",
  });
}

assets.sort((left, right) => right.bytes - left.bytes);

const report = {
  generatedAt: new Date().toISOString(),
  thresholds: {
    largestJavaScriptWarningBytes: 900_000,
    largestGzipJavaScriptWarningBytes: 300_000,
  },
  assets,
  summary: {
    largestAsset: assets[0] ?? null,
    largestJavaScript:
      assets.find((asset) => asset.kind === "javascript") ?? null,
    totalBytes: assets.reduce((total, asset) => total + asset.bytes, 0),
    totalGzipBytes: assets.reduce((total, asset) => total + asset.gzipBytes, 0),
  },
};

await mkdir(outputDir, { recursive: true });
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(`KOMOREBI_BUNDLE_SIZE_REPORT ${outputPath}`);
console.log(JSON.stringify(report.summary, null, 2));
