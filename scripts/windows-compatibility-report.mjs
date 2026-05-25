import { execFile } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const root = process.cwd();
const outputDir = path.join(root, "test-results", "compat");
const outputPath = path.join(outputDir, "windows-compatibility.json");

const packageJson = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
const rustToolchain = await readFile(path.join(root, "rust-toolchain.toml"), "utf8");
const tauriConfig = JSON.parse(
  await readFile(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);

const commandResults = Object.fromEntries(
  await Promise.all(
    [
      ["node", ["--version"]],
      ["pnpm", ["--version"]],
      ["rustc", ["--version"]],
      ["cargo", ["--version"]],
      ["where", ["cl"]],
    ].map(async ([command, args]) => [command, await run(command, args)]),
  ),
);

const webView2 = await queryWebView2();

const report = {
  generatedAt: new Date().toISOString(),
  host: {
    arch: os.arch(),
    platform: os.platform(),
    release: os.release(),
    type: os.type(),
    version: os.version(),
  },
  packagePins: {
    node: packageJson.engines?.node ?? null,
    packageManager: packageJson.packageManager ?? null,
    pnpm: packageJson.engines?.pnpm ?? null,
    rustToolchain: extractRustChannel(rustToolchain),
  },
  toolCommands: commandResults,
  tauri: {
    identifier: tauriConfig.identifier,
    productName: tauriConfig.productName,
    version: tauriConfig.version,
    windows: tauriConfig.app?.windows ?? [],
  },
  webView2,
  supportPosture: {
    targetOs: ["Windows 10", "Windows 11"],
    credentialBackend: "Windows Credential Manager / DPAPI",
    webviewRuntime: "Microsoft Edge WebView2 Evergreen Runtime",
    installerVerification: "P7 deferred; no installer matrix has been run",
  },
};

await mkdir(outputDir, { recursive: true });
await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(`KOMOREBI_WINDOWS_COMPATIBILITY_REPORT ${outputPath}`);
console.log(JSON.stringify(report.supportPosture, null, 2));

async function run(command, args) {
  try {
    const { stdout, stderr } = await execFileAsync(command, args, {
      cwd: root,
      windowsHide: true,
      timeout: 30_000,
    });
    return {
      ok: true,
      stdout: stdout.trim(),
      stderr: stderr.trim(),
    };
  } catch (error) {
    return {
      ok: false,
      stdout: error.stdout?.trim?.() ?? "",
      stderr: error.stderr?.trim?.() ?? error.message,
    };
  }
}

async function queryWebView2() {
  if (os.platform() !== "win32") {
    return {
      installed: false,
      reason: "not-windows",
    };
  }

  const registryQueries = [
    [
      "HKCU\\Software\\Microsoft\\EdgeUpdate\\Clients\\{F1A9F461-F1B3-4D4B-95A2-DBA9E1D19B2A}",
      "/v",
      "pv",
    ],
    [
      "HKLM\\Software\\Microsoft\\EdgeUpdate\\Clients\\{F1A9F461-F1B3-4D4B-95A2-DBA9E1D19B2A}",
      "/v",
      "pv",
    ],
    [
      "HKLM\\Software\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F1A9F461-F1B3-4D4B-95A2-DBA9E1D19B2A}",
      "/v",
      "pv",
    ],
  ];

  const results = [];
  for (const args of registryQueries) {
    results.push(await run("reg", ["query", ...args]));
  }

  const version = results
    .map((result) => result.stdout.match(/pv\s+REG_SZ\s+([^\s]+)/i)?.[1])
    .find(Boolean);

  return {
    installed: Boolean(version),
    registryResults: results,
    version: version ?? null,
  };
}

function extractRustChannel(toolchainToml) {
  return toolchainToml.match(/channel\s*=\s*"([^"]+)"/)?.[1] ?? null;
}
