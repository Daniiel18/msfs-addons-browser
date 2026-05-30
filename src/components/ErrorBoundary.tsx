import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
  info: ErrorInfo | null;
}

/**
 * Last-resort runtime safety net. Without it, a throw inside any component
 * leaves React rendering nothing and the Tauri window appears "empty" —
 * that made Phase 2 debugging much harder than it needed to be. Surfacing
 * the stack directly in the UI pays for itself the first time it fires.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, info: null };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ info });
    console.error("[ErrorBoundary]", error, info);
  }

  handleReload = () => {
    this.setState({ error: null, info: null });
    if (typeof window !== "undefined") window.location.reload();
  };

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="min-h-screen bg-slate-950 p-6 text-slate-100">
        <div className="mx-auto max-w-3xl rounded-2xl border border-red-500/40 bg-red-500/5 p-6">
          <h1 className="text-lg font-semibold text-red-300">
            La interfaz se cayó en runtime
          </h1>
          <p className="mt-1 text-sm text-slate-300">
            Este panel se muestra porque un componente lanzó una excepción. Abre la
            consola (F12) para el stack completo y pulsa Recargar para reintentar.
          </p>
          <pre className="mt-4 max-h-64 overflow-auto rounded-lg bg-slate-900 p-3 text-xs text-red-200">
            {error.message}
            {"\n\n"}
            {error.stack}
          </pre>
          {info?.componentStack && (
            <pre className="mt-3 max-h-64 overflow-auto rounded-lg bg-slate-900 p-3 text-xs text-slate-300">
              {info.componentStack}
            </pre>
          )}
          <button
            onClick={this.handleReload}
            className="mt-4 rounded-lg border border-slate-700 bg-slate-800 px-3 py-1.5 text-xs font-medium text-slate-100 hover:border-brand-500/50"
          >
            Recargar
          </button>
        </div>
      </div>
    );
  }
}
