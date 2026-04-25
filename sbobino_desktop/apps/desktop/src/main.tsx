import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { ErrorBoundary } from "./lib/ErrorBoundary";
import {
  fetchSettingsSnapshot,
  readSetupReport,
} from "./lib/tauri";
import { useAppStore } from "./state/useAppStore";
import "./styles.css";

const standaloneSettingsWindow =
  new URLSearchParams(window.location.search).get("window") === "settings";

// Surface async crashes that previously vanished into a white window. With
// devtools enabled in tauri.conf.json the user can press cmd+option+i and
// read these in the console.
window.addEventListener("error", (event) => {
  // eslint-disable-next-line no-console
  console.error("[window.error]", event.error ?? event.message, event);
});
window.addEventListener("unhandledrejection", (event) => {
  // eslint-disable-next-line no-console
  console.error("[unhandledrejection]", event.reason);
});

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

async function renderApp(): Promise<void> {
  let initialBootstrap:
    | {
      runtimeHealth: null;
      provisioning: null;
      modelCatalog: null;
      startupRequirementsLoaded: boolean;
      setupReport: Awaited<ReturnType<typeof readSetupReport>> | null;
    }
    | undefined;

  try {
    const [settingsResult, setupReportResult] = await Promise.allSettled([
      fetchSettingsSnapshot(),
      readSetupReport(),
    ]);

    if (settingsResult.status !== "fulfilled") {
      throw settingsResult.reason;
    }

    useAppStore.setState({ settings: settingsResult.value });

    const setupReport = setupReportResult.status === "fulfilled"
      ? setupReportResult.value
      : null;

    initialBootstrap = {
      runtimeHealth: null,
      provisioning: null,
      modelCatalog: null,
      startupRequirementsLoaded: Boolean(setupReport?.trusted_for_fast_start),
      setupReport,
    };
  } catch {
    initialBootstrap = undefined;
  }

  root.render(
    <React.StrictMode>
      <ErrorBoundary>
        <App
          standaloneSettingsWindow={standaloneSettingsWindow}
          initialBootstrap={initialBootstrap}
        />
      </ErrorBoundary>
    </React.StrictMode>,
  );
}

void renderApp();
