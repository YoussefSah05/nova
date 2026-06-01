//! Render-error boundary for the browse content. A thrown error in the rails /
//! grid tree shows a retry card instead of silently blanking the whole window.

import { Component, type ReactNode } from "react";
import { AlertTriangleIcon, RotateCwIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";

interface Props {
  children: ReactNode;
  /// Changing this key resets the boundary (e.g. on tab/source change).
  resetKey?: unknown;
}

interface State {
  error: Error | null;
}

export class BrowseBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidUpdate(prev: Props) {
    if (prev.resetKey !== this.props.resetKey && this.state.error) {
      this.setState({ error: null });
    }
  }

  render() {
    if (this.state.error) {
      return (
        <Card className="flex flex-col items-center justify-center gap-3 px-6 py-16 text-center">
          <AlertTriangleIcon className="h-8 w-8 text-muted-foreground" aria-hidden />
          <div>
            <p className="text-sm font-medium text-foreground">This view hit an error</p>
            <p className="mt-1 max-w-md text-sm text-muted-foreground">{this.state.error.message}</p>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => this.setState({ error: null })}
            className="gap-2"
          >
            <RotateCwIcon className="h-4 w-4" aria-hidden /> Try again
          </Button>
        </Card>
      );
    }
    return this.props.children;
  }
}
