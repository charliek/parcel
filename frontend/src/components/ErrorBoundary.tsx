import { Component, type ErrorInfo, type ReactNode } from 'react';
import { Button } from '@/components/ui/button';

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/** Catches render-time errors and shows a recoverable fallback. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // biome-ignore lint/suspicious/noConsole: surface unexpected render errors.
    console.error('Unhandled UI error:', error, info);
  }

  handleReset = () => {
    this.setState({ error: null });
    window.location.assign('/');
  };

  render() {
    if (this.state.error) {
      return (
        <div className="flex min-h-dvh items-center justify-center bg-background px-4">
          <div className="max-w-md space-y-4 text-center">
            <h1 className="font-semibold text-2xl">Something broke</h1>
            <p className="text-muted-foreground text-sm">
              An unexpected error occurred while rendering this page.
            </p>
            <pre className="overflow-x-auto rounded-md border bg-muted/50 p-3 text-left text-muted-foreground text-xs">
              {this.state.error.message}
            </pre>
            <Button onClick={this.handleReset}>Reload Parcel</Button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
