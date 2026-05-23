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
      csp?: unknown;
      devCsp?: unknown;
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

const forbiddenAttachmentOpenCommands = [
  "open_attachment",
  "preview_attachment",
  "save_attachment",
  "download_attachment",
  "extract_attachment",
];

const expectedProductionCsp = [
  "default-src 'self'",
  "script-src 'self'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:",
  "font-src 'self'",
  "connect-src 'self' ipc: http://ipc.localhost",
  "media-src 'none'",
  "object-src 'none'",
  "frame-src 'none'",
  "worker-src 'none'",
  "child-src 'none'",
  "form-action 'none'",
  "base-uri 'none'",
  "frame-ancestors 'none'",
  "manifest-src 'self'",
].join("; ");

const expectedDevelopmentCsp = [
  "default-src 'self' http://localhost:1420",
  "script-src 'self' 'unsafe-eval' http://localhost:1420",
  "style-src 'self' 'unsafe-inline' http://localhost:1420",
  "img-src 'self' data: blob: http://localhost:1420",
  "font-src 'self' data: http://localhost:1420",
  "connect-src 'self' ipc: http://ipc.localhost http://localhost:1420 ws://localhost:1420 ws://localhost:1421",
  "media-src 'none'",
  "object-src 'none'",
  "frame-src 'none'",
  "worker-src 'none'",
  "child-src 'none'",
  "form-action 'none'",
  "base-uri 'none'",
  "frame-ancestors 'none'",
  "manifest-src 'self'",
].join("; ");

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

const accountCommandNames = new Set([
  "create_account",
  "reauth_account",
  "remove_account",
  "update_account_settings",
  "test_account_connection",
  "get_account_auth_status",
]);

const oauthLoopbackCommandNames = new Set([
  "start_oauth_account_setup",
  "start_oauth_account_reauth",
]);

function rustCommandPath(command: string) {
  if (command === "load_mail_shell" || command === "search_mail") {
    return `mail_shell::${command}`;
  }

  if (accountCommandNames.has(command)) {
    return `account_management::${command}`;
  }

  if (oauthLoopbackCommandNames.has(command)) {
    return `oauth_loopback::${command}`;
  }

  return `composer::${command}`;
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

  it("keeps a restrictive production CSP for the main webview", () => {
    const tauriConfig = readJson<TauriConfig>(tauriConfigPath);
    const csp = tauriConfig.app?.security?.csp;

    expect(csp).toBe(expectedProductionCsp);
    expect(csp).not.toContain("*");
    expect(csp).not.toContain("'unsafe-eval'");
    expect(csp).not.toContain("http://localhost");
    expect(csp).not.toContain("https:");
    expect(csp).toContain("connect-src 'self' ipc: http://ipc.localhost");
    expect(csp).toContain("img-src 'self' data: blob:");
    expect(csp).toContain("object-src 'none'");
    expect(csp).toContain("frame-ancestors 'none'");
  });

  it("keeps development CSP limited to local Vite and Tauri IPC", () => {
    const tauriConfig = readJson<TauriConfig>(tauriConfigPath);
    const devCsp = tauriConfig.app?.security?.devCsp;

    expect(devCsp).toBe(expectedDevelopmentCsp);
    expect(devCsp).not.toContain("*");
    expect(devCsp).not.toContain("https:");
    expect(devCsp).toContain("http://localhost:1420");
    expect(devCsp).toContain("ws://localhost:1420");
    expect(devCsp).toContain("ws://localhost:1421");
    expect(devCsp).toContain("connect-src 'self' ipc: http://ipc.localhost");
    expect(devCsp).toContain("object-src 'none'");
    expect(devCsp).toContain("frame-ancestors 'none'");
  });

  it("keeps the main window limited to reviewed mail-shell, composer, account commands, and sync status events", () => {
    const defaultCapability = readCapability("default.json");

    expect(defaultCapability.identifier).toBe("default");
    expect(defaultCapability.windows).toEqual(["main"]);
    expect(defaultCapability.webviews).toBeUndefined();
    expect(defaultCapability.remote).toBeUndefined();
    expect(defaultCapability.permissions).toEqual([
      "core:event:allow-listen",
      "core:event:allow-unlisten",
      "allow-load-mail-shell",
      "allow-search-mail",
      "allow-autosave-composer-draft",
      "allow-discard-composer-draft",
      "allow-stage-composer-attachment",
      "allow-send-composer-email",
      "allow-list-recent-recipients",
      "allow-create-account",
      "allow-reauth-account",
      "allow-remove-account",
      "allow-update-account-settings",
      "allow-test-account-connection",
      "allow-get-account-auth-status",
      "allow-start-oauth-account-setup",
      "allow-start-oauth-account-reauth",
    ]);
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

  it("does not register forbidden plugins or unreviewed custom commands", () => {
    const cargoToml = readFileSync(cargoTomlPath, "utf8");
    const buildScript = readFileSync(buildScriptPath, "utf8");
    const libRs = readFileSync(libRsPath, "utf8");
    const rustSurface = `${cargoToml}\n${buildScript}\n${libRs}`;

    for (const plugin of forbiddenRustPlugins) {
      expect(rustSurface).not.toContain(plugin);
    }

    for (const command of [
      "load_mail_shell",
      "search_mail",
      "autosave_composer_draft",
      "discard_composer_draft",
      "stage_composer_attachment",
      "send_composer_email",
      "list_recent_recipients",
      "create_account",
      "reauth_account",
      "remove_account",
      "update_account_settings",
      "test_account_connection",
      "get_account_auth_status",
      "start_oauth_account_setup",
      "start_oauth_account_reauth",
    ]) {
      expect(buildScript).toContain(`"${command}"`);
      expect(libRs).toContain(rustCommandPath(command));
    }
    expect(buildScript).toContain(".commands(APP_COMMANDS)");
    expect(libRs.match(/generate_handler!\[/g)).toHaveLength(1);
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

  it("does not expose attachment open, save, download, or extraction surfaces", () => {
    const packageJson = readFileSync(packageJsonPath, "utf8");
    const buildScript = readFileSync(buildScriptPath, "utf8");
    const libRs = readFileSync(libRsPath, "utf8");

    expect(packageJson).not.toContain("@tauri-apps/plugin-opener");
    for (const command of forbiddenAttachmentOpenCommands) {
      expect(buildScript).not.toContain(`"${command}"`);
      expect(libRs).not.toContain(command);
    }

    for (const fileName of frontendSourceFileNames) {
      const source = readFileSync(resolve(frontendSourceDir, String(fileName)), "utf8");

      expect(source).not.toContain("@tauri-apps/plugin-opener");
      expect(source).not.toMatch(/\bopenPath\s*\(/);
      expect(source).not.toMatch(/\brevealItemInDir\s*\(/);
    }
  });
});
