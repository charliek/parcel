import { useEffect } from 'react';
import { BrowserRouter, Route, Routes, useLocation } from 'react-router-dom';
import { BootGate } from '@/components/auth/BootGate';
import { RequireAuth } from '@/components/auth/RequireAuth';
import { CliPage } from '@/pages/CliPage';
import { DevicePage } from '@/pages/DevicePage';
import { HomePage } from '@/pages/HomePage';
import { NewParcelPage } from '@/pages/NewParcelPage';
import { SignInPage } from '@/pages/SignInPage';
import { CatchAllPage } from '@/pages/SlugViewerPage';
import { TagsPage } from '@/pages/TagsPage';
import { TokensPage } from '@/pages/TokensPage';
import { ViewerPage } from '@/pages/ViewerPage';

/** Start each page at the top (BrowserRouter has no built-in scroll restoration). */
function ScrollToTop() {
  const { pathname } = useLocation();
  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll on path change only.
  useEffect(() => {
    window.scrollTo(0, 0);
  }, [pathname]);
  return null;
}

export default function App() {
  return (
    // Address changes render right away, not as transitions: the dashboard's
    // filters and the viewer's `?page=` are controlled by the address, and a
    // deferred render would leave a checkbox or a key handler a step behind.
    <BrowserRouter useTransitions={false}>
      <ScrollToTop />
      <BootGate>
        <Routes>
          {/* Sign-in itself is the server's (/auth/*); it sends people here to
              pick a provider, or when a sign-in failed. */}
          <Route path="/signin" element={<SignInPage />} />

          {/* Optional auth: the landing page or the dashboard. */}
          <Route path="/" element={<HomePage />} />

          {/* Optional auth: public parcels render for anonymous visitors. */}
          <Route path="/p/:id" element={<ViewerPage />} />
          <Route path="/p/:id/v/:version" element={<ViewerPage />} />

          {/* Account-only pages. */}
          <Route
            path="/new"
            element={
              <RequireAuth signInDescription="Sign in to publish a new parcel.">
                <NewParcelPage />
              </RequireAuth>
            }
          />
          <Route
            path="/tags"
            element={
              <RequireAuth signInDescription="Sign in to manage your tags.">
                <TagsPage />
              </RequireAuth>
            }
          />
          <Route
            path="/tokens"
            element={
              <RequireAuth signInDescription="Sign in to manage your API tokens.">
                <TokensPage />
              </RequireAuth>
            }
          />
          <Route
            path="/cli"
            element={
              <RequireAuth signInDescription="Sign in to set up the Parcel CLI and agent skill.">
                <CliPage />
              </RequireAuth>
            }
          />
          <Route
            path="/device"
            element={
              <RequireAuth
                signInTitle="Sign in to connect the CLI"
                signInDescription="Sign in to approve the login request from your terminal. Your code is kept."
              >
                <DevicePage />
              </RequireAuth>
            }
          />

          {/* Clean parcel URLs (`/@handle/slug`, optional auth like `/p/:id`),
              else the not-found page. */}
          <Route path="*" element={<CatchAllPage />} />
        </Routes>
      </BootGate>
    </BrowserRouter>
  );
}
