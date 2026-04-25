import React from "react";

type ErrorBoundaryState = {
  error: Error | null;
  componentStack: string | null;
};

export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  ErrorBoundaryState
> {
  state: ErrorBoundaryState = { error: null, componentStack: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error, componentStack: null };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    // Log to the webview console so opening DevTools (cmd+option+i) reveals
    // the full stack. Without this, an uncaught render error wipes the UI
    // (white window) and there is no on-screen trace to act on.
    // eslint-disable-next-line no-console
    console.error("[ErrorBoundary] uncaught", error, info.componentStack);
    this.setState({ error, componentStack: info.componentStack ?? null });
  }

  private reload = (): void => {
    window.location.reload();
  };

  private copyDetails = (): void => {
    const text = [
      this.state.error?.name ?? "Error",
      this.state.error?.message ?? "",
      "",
      "Stack:",
      this.state.error?.stack ?? "",
      "",
      "Component stack:",
      this.state.componentStack ?? "",
    ].join("\n");
    void navigator.clipboard?.writeText(text).catch(() => {});
  };

  render(): React.ReactNode {
    if (!this.state.error) {
      return this.props.children;
    }

    return (
      <div
        style={{
          fontFamily:
            "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
          padding: 24,
          maxWidth: 720,
          margin: "48px auto",
          color: "#1d1d1f",
          background: "#fff",
          borderRadius: 12,
          boxShadow: "0 12px 40px rgba(0,0,0,0.08)",
        }}
      >
        <h2 style={{ marginTop: 0 }}>Something went wrong</h2>
        <p style={{ color: "#555", lineHeight: 1.5 }}>
          The Sbobino UI hit an unexpected error. The app process is still
          running; reloading the window usually recovers without losing your
          transcripts. If the problem keeps happening, copy the details below
          and report it.
        </p>
        <pre
          style={{
            background: "#f6f6f7",
            padding: 12,
            borderRadius: 8,
            fontSize: 12,
            maxHeight: 280,
            overflow: "auto",
          }}
        >
{`${this.state.error.name}: ${this.state.error.message}

${this.state.error.stack ?? ""}

Component stack:${this.state.componentStack ?? ""}`}
        </pre>
        <div style={{ display: "flex", gap: 12, marginTop: 16 }}>
          <button
            type="button"
            onClick={this.reload}
            style={{
              background: "#0a84ff",
              color: "white",
              border: "none",
              padding: "10px 16px",
              borderRadius: 8,
              cursor: "pointer",
            }}
          >
            Reload window
          </button>
          <button
            type="button"
            onClick={this.copyDetails}
            style={{
              background: "#e5e5ea",
              color: "#1d1d1f",
              border: "none",
              padding: "10px 16px",
              borderRadius: 8,
              cursor: "pointer",
            }}
          >
            Copy error details
          </button>
        </div>
      </div>
    );
  }
}
