import { useTranslation } from "react-i18next";

export function MailClientTitleBar({
  scopeLabel,
  title,
}: {
  scopeLabel: string;
  title: string;
}) {
  const { t } = useTranslation();

  return (
    <header
      className="window-titlebar"
      data-tauri-drag-region
      onDoubleClick={() => void runWindowCommand("toggleMaximize")}
    >
      <div className="window-titlebar__brand" data-tauri-drag-region>
        <span className="window-titlebar__mark" aria-hidden="true">
          K
        </span>
        <div data-tauri-drag-region>
          <strong data-tauri-drag-region>Komorebi</strong>
          <span data-tauri-drag-region>{t("app.titlebar.subtitle")}</span>
        </div>
      </div>
      <div className="window-titlebar__context" data-tauri-drag-region>
        <span data-tauri-drag-region>{scopeLabel}</span>
        <strong data-tauri-drag-region>{title}</strong>
      </div>
      <div
        className="window-titlebar__controls"
        aria-label={t("app.titlebar.windowControls")}
      >
        <button
          type="button"
          aria-label={t("app.titlebar.minimize")}
          onClick={() => void runWindowCommand("minimize")}
        >
          -
        </button>
        <button
          type="button"
          aria-label={t("app.titlebar.maximize")}
          onClick={() => void runWindowCommand("toggleMaximize")}
        >
          □
        </button>
        <button
          type="button"
          aria-label={t("app.titlebar.close")}
          onClick={() => void runWindowCommand("close")}
        >
          ×
        </button>
      </div>
    </header>
  );
}

async function runWindowCommand(
  command: "close" | "minimize" | "toggleMaximize",
) {
  if (!("__TAURI_INTERNALS__" in window)) {
    return;
  }

  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const appWindow = getCurrentWindow();
  if (command === "close") {
    await appWindow.close();
  } else if (command === "minimize") {
    await appWindow.minimize();
  } else {
    await appWindow.toggleMaximize();
  }
}
