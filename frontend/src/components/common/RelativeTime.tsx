import { useEffect, useState } from 'react';
import { formatDateTime, formatRelativeTime } from '@/lib/format';

/** A `<time>` showing relative time, with the absolute time on hover. Re-renders each minute. */
export function RelativeTime({ value, className }: { value: string; className?: string }) {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 60_000);
    return () => clearInterval(t);
  }, []);
  return (
    <time dateTime={value} title={formatDateTime(value)} className={className}>
      {formatRelativeTime(value, now)}
    </time>
  );
}
