import { ChevronDown, History } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { formatDateTime, formatRelativeTime } from '@/lib/format';
import type { VersionSummary } from '@/lib/types';

/** Path for a version: the bare parcel URL for the current one, `/v/N` otherwise. */
export function versionPath(id: string, version: number, current: number): string {
  return version === current ? `/p/${id}` : `/p/${id}/v/${version}`;
}

/** Version selector, newest first. */
export function VersionMenu({
  parcelId,
  versions,
  viewing,
  current,
}: {
  parcelId: string;
  /** Newest first. */
  versions: VersionSummary[];
  viewing: number;
  current: number;
}) {
  const navigate = useNavigate();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 shrink-0 gap-1 px-2 font-mono text-xs"
          aria-label={`Version ${viewing}, choose another version`}
        >
          <History className="h-3.5 w-3.5 text-muted-foreground" aria-hidden="true" />v{viewing}
          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-[min(24rem,70dvh)] w-72 overflow-y-auto">
        <DropdownMenuLabel>Versions</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={String(viewing)}
          onValueChange={(value) => navigate(versionPath(parcelId, Number(value), current))}
        >
          {versions.map((v) => (
            <DropdownMenuRadioItem
              key={v.version}
              value={String(v.version)}
              className="items-start"
            >
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="flex items-center gap-2">
                  <span className="font-medium font-mono text-xs">v{v.version}</span>
                  {v.version === current && (
                    <span className="rounded bg-primary/12 px-1.5 text-[10px] text-primary uppercase tracking-wide">
                      Current
                    </span>
                  )}
                  <span
                    className="ml-auto text-muted-foreground text-xs"
                    title={formatDateTime(v.created_at)}
                  >
                    {formatRelativeTime(v.created_at)}
                  </span>
                </span>
                {v.label && (
                  <span className="truncate text-muted-foreground text-xs">{v.label}</span>
                )}
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
