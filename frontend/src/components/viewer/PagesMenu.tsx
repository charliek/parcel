import { ChevronDown, FileText } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { pageLabel } from '@/lib/refs';
import type { PageEntry } from '@/lib/types';

/** The pages of the version in view (shown when it has more than one), entry first. */
export function PagesMenu({
  pages,
  current,
  onOpen,
}: {
  pages: PageEntry[];
  /** Path of the page in the frame. */
  current: string;
  onOpen: (path: string) => void;
}) {
  const shown = pages.find((p) => p.path === current);
  const label = shown ? pageLabel(shown) : current;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 min-w-0 max-w-48 shrink gap-1 px-2 text-xs"
          aria-label={`Page: ${label}. Choose another page`}
          data-testid="pages-menu"
        >
          <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="truncate">{label}</span>
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-[min(24rem,70dvh)] w-72 overflow-y-auto">
        <DropdownMenuLabel>Pages</DropdownMenuLabel>
        <DropdownMenuRadioGroup value={current} onValueChange={onOpen}>
          {pages.map((page) => (
            <DropdownMenuRadioItem key={page.path} value={page.path} className="items-start">
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate">{pageLabel(page)}</span>
                <span className="truncate font-mono text-muted-foreground text-xs">
                  {page.path}
                </span>
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
