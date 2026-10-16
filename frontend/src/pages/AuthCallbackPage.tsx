/**
 * OAuth2 callback: exchange the authorization code for tokens, then return to
 * where the user started. Rendered outside any auth gate so it runs even when
 * there is no session yet.
 */

import { AlertCircle } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { FullPageMessage, Splash } from '@/components/common/FullPage';
import { Button } from '@/components/ui/button';
import { beginLogin, rememberSignedIn } from '@/lib/auth';
import {
  clearSilentReauthGuard,
  exchangeCode,
  markSilentReauthTried,
  takePkceTx,
} from '@/lib/oauth';
import { safeReturnTo } from '@/lib/return-to';
import { useTokenStore } from '@/lib/token-store';
import { useAuthStore } from '@/stores/auth.store';

/** OAuth error codes that just mean "no usable slauth session" (expected for prompt=none). */
const QUIET_ERRORS = new Set([
  'login_required',
  'consent_required',
  'interaction_required',
  'account_selection_required',
]);

export function AuthCallbackPage() {
  const navigate = useNavigate();
  const ran = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [returnTo, setReturnTo] = useState('/');

  useEffect(() => {
    if (ran.current) return;
    ran.current = true;

    const params = new URLSearchParams(window.location.search);
    const code = params.get('code');
    const state = params.get('state');
    const errorParam = params.get('error');
    const errorDescription = params.get('error_description');
    const tx = takePkceTx();
    const destination = safeReturnTo(tx?.returnTo);
    setReturnTo(destination);

    (async () => {
      // A silent attempt that found no session, or an explicit cancel: go back
      // to where the user was; that page shows its signed-out state.
      if (errorParam) {
        markSilentReauthTried();
        useAuthStore.getState().setAnonymous();
        if (!tx?.silent && !QUIET_ERRORS.has(errorParam)) {
          toast.error(
            errorParam === 'access_denied'
              ? 'Sign-in was cancelled.'
              : (errorDescription ?? 'Sign-in failed.'),
          );
        }
        navigate(destination, { replace: true });
        return;
      }
      if (!code || !state || !tx || state !== tx.state) {
        setError('This sign-in response is invalid or was already used. Please try again.');
        return;
      }
      try {
        const tokens = await exchangeCode(code, tx.verifier);
        useTokenStore.getState().setTokens(tokens);
        clearSilentReauthGuard();
        rememberSignedIn();
        void useAuthStore.getState().reload();
        navigate(destination, { replace: true });
      } catch {
        setError('Sign-in failed while exchanging the authorization code. Please try again.');
      }
    })();
  }, [navigate]);

  if (error) {
    return (
      <FullPageMessage
        icon={AlertCircle}
        tone="destructive"
        title="Sign-in didn't complete"
        description={error}
        actions={
          <>
            <Button onClick={() => void beginLogin(returnTo)}>Try again</Button>
            <Button variant="outline" onClick={() => navigate(returnTo, { replace: true })}>
              Go back
            </Button>
          </>
        }
      />
    );
  }

  return <Splash message="Signing you in…" />;
}
