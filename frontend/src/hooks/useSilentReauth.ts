import { useEffect, useState } from 'react';
import { maybeStartSilentReauth } from '@/lib/oauth';
import { currentReturnTo } from '@/lib/return-to';

export type SilentReauthState = 'idle' | 'redirecting' | 'unavailable';

/**
 * Kick off the once-per-session silent (prompt=none) re-auth when `want`
 * becomes true. While the redirect is in flight the caller should keep
 * showing a splash; `unavailable` means no attempt could be made.
 */
export function useSilentReauth(want: boolean): SilentReauthState {
  const [state, setState] = useState<SilentReauthState>('idle');
  useEffect(() => {
    if (!want || state !== 'idle') return;
    const attempt = maybeStartSilentReauth(currentReturnTo());
    if (!attempt) {
      setState('unavailable');
      return;
    }
    setState('redirecting');
    attempt.catch(() => setState('unavailable'));
  }, [want, state]);
  return state;
}
