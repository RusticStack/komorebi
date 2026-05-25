import { execFile, spawn } from "node:child_process";
import { mkdir, rm, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const root = process.cwd();
const exePath = path.join(root, "src-tauri", "target", "release", "komorebi.exe");
const outputDir = path.join(root, "test-results", "perf");
const outputPath = path.join(outputDir, "release-memory.json");

await stat(exePath).catch((error) => {
  throw new Error(
    `Release executable is unavailable at ${exePath}; run pnpm tauri build --no-bundle first. ${error.message}`,
  );
});

const appDataDir = path.join(os.tmpdir(), `komorebi-memory-${Date.now()}`);
const child = spawn(exePath, [], {
  cwd: root,
  env: {
    ...process.env,
    KOMOREBI_MEMORY_MEASUREMENT: "1",
    LOCALAPPDATA: appDataDir,
  },
  stdio: "ignore",
  windowsHide: true,
});

const samples = [];
try {
  await delay(3000);
  for (let index = 0; index < 8; index += 1) {
    samples.push(await sampleProcess(child.pid));
    await delay(1000);
  }
} finally {
  if (!child.killed) {
    child.kill();
  }
  await rm(appDataDir, { force: true, recursive: true }).catch(() => {});
}

const validSamples = samples.filter((sample) => sample.ok);
const report = {
  generatedAt: new Date().toISOString(),
  executable: exePath,
  scenario: "release-idle-accountless-shell",
  samples,
  summary: {
    maxWorkingSetBytes: Math.max(
      0,
      ...validSamples.map((sample) => sample.workingSetBytes),
    ),
    maxPrivateMemoryBytes: Math.max(
      0,
      ...validSamples.map((sample) => sample.privateMemoryBytes),
    ),
    sampleCount: validSamples.length,
  },
  caveats: [
    "This measures the accountless release shell only.",
    "Large-mailbox, search, composer, sync, and attachment-heavy memory remain covered by browser/backend benchmarks until a Tauri-shell fixture harness is promoted from P7.",
  ],
};

await mkdir(outputDir, { recursive: true });
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(`KOMOREBI_RELEASE_MEMORY_REPORT ${outputPath}`);
console.log(JSON.stringify(report.summary, null, 2));

async function sampleProcess(pid) {
  const command = [
    "$process = Get-Process -Id",
    String(pid),
    "-ErrorAction SilentlyContinue;",
    "if ($null -eq $process) { exit 3 };",
    "[Console]::Out.Write(($process | Select-Object Id,ProcessName,WorkingSet64,PrivateMemorySize64 | ConvertTo-Json -Compress))",
  ].join(" ");
  try {
    const { stdout } = await execFileAsync(
      "powershell",
      ["-NoProfile", "-Command", command],
      { timeout: 15_000, windowsHide: true },
    );
    const parsed = JSON.parse(stdout);
    return {
      ok: true,
      privateMemoryBytes: parsed.PrivateMemorySize64,
      processId: parsed.Id,
      processName: parsed.ProcessName,
      workingSetBytes: parsed.WorkingSet64,
    };
  } catch (error) {
    return {
      error: error.message,
      ok: false,
    };
  }
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
