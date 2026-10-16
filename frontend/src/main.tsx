import { QueryClientProvider } from '@tanstack/react-query';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import { ErrorBoundary } from './components/ErrorBoundary';
import { Toaster } from './components/ui/sonner';
import { TooltipProvider } from './components/ui/tooltip';
import { queryClient } from './lib/query-client';
// Side effects: apply the theme early and wire cross-tab auth sync.
import './stores/theme.store';
import './stores/auth.store';
import './index.css';

const rootElement = document.getElementById('root');
if (rootElement) {
  createRoot(rootElement).render(
    <StrictMode>
      <ErrorBoundary>
        <QueryClientProvider client={queryClient}>
          <TooltipProvider delayDuration={300}>
            <App />
            <Toaster position="bottom-right" closeButton />
          </TooltipProvider>
        </QueryClientProvider>
      </ErrorBoundary>
    </StrictMode>,
  );
}
