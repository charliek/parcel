import { useState } from 'react';
import { toast } from 'sonner';
import { logout } from '@/lib/auth';
import { errorMessage } from '@/lib/http';

/** Sign out (here, or `everywhere`), with a toast if the server can't be reached. */
export function useSignOut() {
  const [pending, setPending] = useState(false);
  const signOut = (everywhere = false) => {
    setPending(true);
    logout({ everywhere }).catch((err) => {
      setPending(false);
      toast.error('Could not sign out', { description: errorMessage(err) });
    });
  };
  return { signOut, pending };
}
