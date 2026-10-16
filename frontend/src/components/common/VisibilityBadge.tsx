import { Globe, Lock } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import type { Visibility } from '@/lib/types';
import { cn } from '@/lib/utils';

/** `Public` (anyone with the link) or `Private` (owner only). */
export function VisibilityBadge({
  visibility,
  className,
}: {
  visibility: Visibility;
  className?: string;
}) {
  const isPublic = visibility === 'public';
  return (
    <Badge
      variant={isPublic ? 'success' : 'muted'}
      className={cn(className)}
      title={isPublic ? 'Anyone with the link can view' : 'Only you can view'}
    >
      {isPublic ? <Globe aria-hidden="true" /> : <Lock aria-hidden="true" />}
      {isPublic ? 'Public' : 'Private'}
    </Badge>
  );
}
