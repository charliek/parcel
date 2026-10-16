import { Globe, Lock, Users } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import type { Visibility } from '@/lib/types';
import { cn } from '@/lib/utils';
import { VISIBILITY } from '@/lib/visibility';

/** The icon for each visibility, shared by badges, pickers and menus. */
export const VISIBILITY_ICON: Record<Visibility, typeof Lock> = {
  private: Lock,
  internal: Users,
  public: Globe,
};

const VARIANT = { private: 'muted', internal: 'accent', public: 'success' } as const;

/** `Private`, `Internal` (everyone signed in) or `Public` (anyone with the link). */
export function VisibilityBadge({
  visibility,
  className,
}: {
  visibility: Visibility;
  className?: string;
}) {
  // An unknown value (a newer server) reads as the narrowest.
  const known: Visibility = visibility in VISIBILITY ? visibility : 'private';
  const Icon = VISIBILITY_ICON[known];
  return (
    <Badge variant={VARIANT[known]} className={cn(className)} title={VISIBILITY[known].who}>
      <Icon aria-hidden="true" />
      {VISIBILITY[known].badge}
    </Badge>
  );
}
