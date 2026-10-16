import { toast } from 'sonner';
import { copyText } from '@/components/common/CopyButton';
import { errorMessage } from '@/lib/http';
import type { ParcelSummary, Visibility } from '@/lib/types';
import { visibilityChangedText } from '@/lib/visibility';
import { useUpdateParcel } from './useParcels';

/**
 * Change who can open a parcel, with a toast that says what that means (and
 * offers the link when it opened up). `shown` is the value to display: the
 * one being saved while the request is in flight.
 */
export function useChangeVisibility(parcel: ParcelSummary, link: string) {
  const update = useUpdateParcel(parcel.id);
  const change = (visibility: Visibility) => {
    if (visibility === parcel.visibility) return;
    update.mutate(
      { visibility },
      {
        onSuccess: (summary) => {
          const text = visibilityChangedText(summary.visibility);
          toast.success(text.title, {
            description: text.description,
            action:
              summary.visibility === 'private'
                ? undefined
                : { label: 'Copy link', onClick: () => void copyText(link) },
          });
        },
        onError: (err) =>
          toast.error('Could not change who can open it', { description: errorMessage(err) }),
      },
    );
  };
  const shown = update.isPending
    ? (update.variables?.visibility ?? parcel.visibility)
    : parcel.visibility;
  return { change, shown, pending: update.isPending };
}
