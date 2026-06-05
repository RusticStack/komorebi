import { spawn } from "node:child_process";
import { access, mkdir, writeFile } from "node:fs/promises";
import { createConnection } from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const binaryPath = path.join(
  repoRoot,
  "src-tauri",
  "target",
  "release",
  "komorebi.exe",
);
const artifactsDir = path.join(repoRoot, "test-results", "tauri-webdriver");
const host = "127.0.0.1";
const port = Number.parseInt(process.env.KOMOREBI_TAURI_DRIVER_PORT ?? "4444", 10);
const driverUrl = `http://${host}:${port}`;
const tauriDriverBin = process.env.KOMOREBI_TAURI_DRIVER_BIN ?? "tauri-driver";
const nativeDriver = process.env.KOMOREBI_NATIVE_WEBDRIVER;
const localNativeDriver = path.join(
  repoRoot,
  "test-results",
  "tools",
  "msedgedriver.exe",
);

function delay(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

function commandExists(command, args = ["--version"]) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { stdio: "ignore" });
    child.on("error", () => resolve(false));
    child.on("exit", (code) => resolve(code === 0));
  });
}

async function waitForPort(timeoutMs) {
  const start = Date.now();
  let lastError;

  while (Date.now() - start < timeoutMs) {
    const connected = await new Promise((resolve) => {
      const socket = createConnection({ host, port });
      socket.once("connect", () => {
        socket.end();
        resolve(true);
      });
      socket.once("error", (error) => {
        lastError = error;
        resolve(false);
      });
    });

    if (connected) {
      return;
    }

    await delay(250);
  }

  throw new Error(
    `tauri-driver did not listen on ${host}:${port}: ${lastError?.message ?? "timeout"}`,
  );
}

async function webdriver(method, route, body) {
  const response = await fetch(`${driverUrl}${route}`, {
    method,
    headers: {
      "content-type": "application/json",
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  const json = text ? JSON.parse(text) : {};

  if (!response.ok || json.value?.error) {
    const message = json.value?.message ?? text ?? response.statusText;
    throw new Error(`${method} ${route} failed: ${message}`);
  }

  return json.value;
}

async function captureFailure(sessionId, label, error) {
  await mkdir(artifactsDir, { recursive: true });

  try {
    const screenshot = await webdriver("GET", `/session/${sessionId}/screenshot`);
    await writeFile(
      path.join(artifactsDir, `${label}.png`),
      Buffer.from(screenshot, "base64"),
    );
  } catch {
    // Best-effort failure artifacts should never hide the original failure.
  }

  try {
    const bodyText = await webdriver("POST", `/session/${sessionId}/execute/sync`, {
      script: "return document.body ? document.body.innerText : '';",
      args: [],
    });
    await writeFile(path.join(artifactsDir, `${label}.txt`), String(bodyText));
  } catch {
    // Best-effort failure artifacts should never hide the original failure.
  }

  await writeFile(path.join(artifactsDir, `${label}.error.txt`), String(error.stack ?? error));
}

async function main() {
  if (process.platform !== "win32") {
    throw new Error("Komorebi Tauri WebDriver smoke currently targets Windows.");
  }

  try {
    await access(binaryPath);
  } catch {
    throw new Error(
      `release executable was not found at ${binaryPath}. Run \`pnpm tauri build --no-bundle\` first.`,
    );
  }

  if (!(await commandExists(tauriDriverBin, ["--help"]))) {
    throw new Error(
      "tauri-driver was not found. Install it with `cargo install tauri-driver --locked`.",
    );
  }

  let resolvedNativeDriver = nativeDriver;
  if (!resolvedNativeDriver) {
    try {
      await access(localNativeDriver);
      resolvedNativeDriver = localNativeDriver;
    } catch {
      resolvedNativeDriver = undefined;
    }
  }

  if (!resolvedNativeDriver && !(await commandExists("msedgedriver"))) {
    throw new Error(
      "msedgedriver was not found on PATH. Install a matching Microsoft Edge Driver or set KOMOREBI_NATIVE_WEBDRIVER.",
    );
  }

  const args = ["--port", String(port)];
  if (resolvedNativeDriver) {
    args.push("--native-driver", resolvedNativeDriver);
  }

  const driver = spawn(tauriDriverBin, args, {
    stdio: ["ignore", "pipe", "pipe"],
  });

  let driverOutput = "";
  driver.stdout.on("data", (chunk) => {
    driverOutput += chunk.toString();
  });
  driver.stderr.on("data", (chunk) => {
    driverOutput += chunk.toString();
  });

  let sessionId;
  try {
    await waitForPort(15_000);

    const session = await webdriver("POST", "/session", {
      capabilities: {
        alwaysMatch: {
          "tauri:options": {
            application: binaryPath,
          },
        },
      },
    });

    sessionId = session.sessionId;

    const result = await webdriver("POST", `/session/${sessionId}/execute/sync`, {
      script: `
        const titleText = document.querySelector('.window-titlebar__brand strong')?.textContent?.trim() ?? '';
        const sidebarBrand = document.querySelector('.brand h1')?.textContent?.trim() ?? '';
        const shellExists = Boolean(document.querySelector('.app-frame .app-shell'));
        const onboardingExists = Boolean(document.querySelector('.mail-onboarding'));
        const settingsButton = document.querySelector('.sidebar-section--footer .settings-button');
        const bridgeExists = Boolean(globalThis.__TAURI_INTERNALS__?.invoke || globalThis.__TAURI__?.core?.invoke);
        return { titleText, sidebarBrand, shellExists, onboardingExists, hasSettingsButton: Boolean(settingsButton), bridgeExists };
      `,
      args: [],
    });

    const failures = [];
    if (result.titleText !== "Komorebi") {
      failures.push(`expected custom titlebar brand Komorebi, got ${JSON.stringify(result.titleText)}`);
    }
    if (result.sidebarBrand !== "Komorebi") {
      failures.push(`expected sidebar brand Komorebi, got ${JSON.stringify(result.sidebarBrand)}`);
    }
    if (!result.shellExists) {
      failures.push("expected app shell DOM to exist");
    }
    if (!result.onboardingExists) {
      failures.push("expected accountless onboarding surface to exist");
    }
    if (!result.hasSettingsButton) {
      failures.push("expected settings/app navigation button to exist");
    }
    if (!result.bridgeExists) {
      failures.push("expected Tauri invoke bridge to be available");
    }

    if (failures.length > 0) {
      throw new Error(failures.join("; "));
    }

    console.log("Tauri WebDriver smoke passed.");
  } catch (error) {
    if (sessionId) {
      await captureFailure(sessionId, "smoke-failure", error);
    }

    if (driverOutput.trim()) {
      console.error(driverOutput.trim());
    }
    throw error;
  } finally {
    if (sessionId) {
      try {
        await webdriver("DELETE", `/session/${sessionId}`);
      } catch {
        // The application may already be closed by the failed session.
      }
    }

    if (driver.exitCode === null && !driver.killed) {
      driver.kill();
    }
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
