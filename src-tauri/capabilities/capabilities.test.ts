import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

type CapabilityPermission =
  | string
  | {
      identifier: string;
      [key: string]: unknown;
    };

type Capability = {
  identifier: string;
  description?: string;
  windows?: string[];
  webviews?: string[];
  remote?: unknown;
  permissions?: CapabilityPermission[];
};

type TauriConfig = {
  app?: {
    security?: {
      capabilities?: unknown;
    };
  };
};

const capabilityDir = resolve("src-tauri", "capabilities");
const tauriConfigPath = resolve("src-tauri", "tauri.conf.json");
const cargoTomlPath = resolve("src-tauri", "Cargo.toml");
const buildScriptPath = resolve("src-tauri", "build.rs");
const libRsPath = resolve("src-tauri", "src", "lib.rs");
const packageJsonPath = resolve("package.json");
const frontendSourceDir = resolve("src");

const forbiddenPermissionPrefixes = [
  "sql:",
  "fs:",
  "filesystem:",
  "opener:",
  "shell:",
  "process:",
  "http:",
];

const forbiddenRustPlugins = [
  "tauri_plugin_sql",
  "tauri-plugin-sql",
  "tauri_plugin_fs",
  "tauri-plugin-fs",
  "tauri_plugin_opener",
  "tauri-plugin-opener",
  "tauri_plugin_shell",
  "tauri-plugin-shell",
  "tauri_plugin_process",
  "tauri-plugin-process",
  "tauri_plugin_http",
  "tauri-plugin-http",
];

const capabilityFileNames = readdirSync(capabilityDir).filter((fileName) =>
  fileName.endsWith(".json"),
);
const frontendSourceFileNames = readdirSync(frontendSourceDir, {
  recursive: true,
}).filter((fileName) => String(fileName).match(/\.(ts|tsx)$/));

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

function permissionIdentifier(permission: CapabilityPermission): string {
  return typeof permission === "string" ? permission : permission.identifier;
}

function readCapability(fileName: string): Capability {
  return readJson<Capability>(resolve(capabilityDir, fileName));
}

describe("Tauri capabilities", () => {
  it("explicitly enables only the reviewed default capability", () => {
    const tauriConfig = readJson<TauriConfig>(tauriConfigPath);

    expect(tauriConfig.app?.security?.capabilities).toEqual(["default"]);
    expect(capabilityFileNames).toEqual(["default.json"]);
  });

  it("keeps the main window without frontend Tauri permissions", () => {
    const defaultCapability = readCapability("default.json");

    expect(defaultCapability.identifier).toBe("default");
    expect(defaultCapability.windows).toEqual(["main"]);
    expect(defaultCapability.webviews).toBeUndefined();
    expect(defaultCapability.remote).toBeUndefined();
    expect(defaultCapability.permissions).toEqual([]);
  });

  it("denies broad local-system and network plugin permissions by absence", () => {
    for (const fileName of capabilityFileNames) {
      const capability = readCapability(fileName);

      expect(capability.windows).not.toContain("*");
      expect(capability.webviews).toBeUndefined();
      expect(capability.remote).toBeUndefined();

      for (const permission of capability.permissions ?? []) {
        const identifier = permissionIdentifier(permission);

        expect(identifier).not.toBe("*");
        expect(identifier).not.toMatch(/(^|:)allow-all$/);
        expect(
          forbiddenPermissionPrefixes.some((prefix) =>
            identifier.startsWith(prefix),
          ),
        ).toBe(false);
      }
    }
  });

  it("does not register forbidden plugins or custom commands", () => {
    const cargoToml = readFileSync(cargoTomlPath, "utf8");
    const buildScript = readFileSync(buildScriptPath, "utf8");
    const libRs = readFileSync(libRsPath, "utf8");
    const rustSurface = `${cargoToml}\n${buildScript}\n${libRs}`;

    for (const plugin of forbiddenRustPlugins) {
      expect(rustSurface).not.toContain(plugin);
    }

    expect(buildScript).toContain("const APP_COMMANDS: &[&str] = &[];");
    expect(buildScript).toContain(".commands(APP_COMMANDS)");
    expect(libRs).not.toMatch(/invoke_handler\s*\(/);
    expect(libRs).not.toContain("tauri::generate_handler!");
  });

  it("keeps frontend SQL unavailable until a read-only policy is implemented", () => {
    const packageJson = readFileSync(packageJsonPath, "utf8");

    expect(packageJson).not.toContain("@tauri-apps/plugin-sql");

    for (const fileName of frontendSourceFileNames) {
      const source = readFileSync(resolve(frontendSourceDir, String(fileName)), "utf8");

      expect(source).not.toContain("@tauri-apps/plugin-sql");
      expect(source).not.toContain("window.__TAURI__.sql");
      expect(source).not.toMatch(/\bDatabase\.load\s*\(/);
    }
  });
});
