import { useEffect } from 'react';

/** Set `document.title` to `"<title> · Parcel"` (or just `Parcel`). */
export function useDocumentTitle(title?: string | null) {
  useEffect(() => {
    document.title = title ? `${title} · Parcel` : 'Parcel';
  }, [title]);
}
