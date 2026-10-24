import {
  Archive,
  ArchiveRestore,
  Download,
  ExternalLink,
  Link2,
  MoreHorizontal,
  Pencil,
  RotateCcw,
  RotateCw,
  Share2,
  Trash2,
  Upload,
} from 'lucide-react';
import type { MouseEvent } from 'react';
import { IconButton } from '@/components/common/IconButton';
import { VISIBILITY_ICON } from '@/components/common/VisibilityBadge';
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { Visibility } from '@/lib/types';
import { visibilityOptions } from '@/lib/visibility';

export interface ViewerActions {
  reload: () => void;
  copyLink: () => void;
  contentUrl: string;
  openInNewTab: (event: MouseEvent<HTMLAnchorElement>) => void;
  downloadEntry: () => void;
  share: () => void;
  edit: () => void;
  uploadVersion: () => void;
  setVisibility: (visibility: Visibility) => void;
  restore: () => void;
  remove: () => void;
  /** Archive, or unarchive when it is archived. */
  toggleArchived: () => void;
}

/**
 * Overflow menu. On phones it also carries the actions that sit in the top
 * bar on wider screens; owners get the parcel's settings; everyone gets the
 * viewer preferences. Public links are left out where the server refuses
 * them (the share dialog says why).
 */
export function ViewerMoreMenu({
  actions,
  isOwner,
  isCurrent,
  archived,
  visibility,
  allowPublic,
  autoReload,
  onAutoReloadChange,
}: {
  actions: ViewerActions;
  isOwner: boolean;
  isCurrent: boolean;
  /** The parcel is archived (the menu offers Unarchive). */
  archived: boolean;
  visibility: Visibility;
  allowPublic: boolean;
  /** Reload the frame when a new version is published. */
  autoReload: boolean;
  onAutoReloadChange: (on: boolean) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <IconButton label="More actions">
          <MoreHorizontal />
        </IconButton>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <div className="md:hidden">
          <DropdownMenuItem onSelect={actions.reload}>
            <RotateCw />
            Reload
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={actions.copyLink}>
            <Link2 />
            Copy link
          </DropdownMenuItem>
          <DropdownMenuItem asChild>
            <a
              href={actions.contentUrl}
              target="_blank"
              rel="noopener noreferrer"
              onClick={actions.openInNewTab}
            >
              <ExternalLink />
              Open in new tab
            </a>
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={actions.downloadEntry}>
            <Download />
            Download page
          </DropdownMenuItem>
          <DropdownMenuSeparator />
        </div>
        {isOwner && (
          <>
            <DropdownMenuItem onSelect={actions.share} className="sm:hidden">
              <Share2 />
              Share…
            </DropdownMenuItem>
            <DropdownMenuLabel>General access</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={visibility}
              onValueChange={(value) => actions.setVisibility(value as Visibility)}
            >
              {visibilityOptions(allowPublic)
                .filter((option) => option.disabledReason === null)
                .map(({ value, label }) => {
                  const Icon = VISIBILITY_ICON[value];
                  return (
                    <DropdownMenuRadioItem key={value} value={value}>
                      <Icon className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
                      {label}
                    </DropdownMenuRadioItem>
                  );
                })}
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={actions.edit}>
              <Pencil />
              Edit details…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={actions.uploadVersion}>
              <Upload />
              Upload new version…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={actions.toggleArchived}>
              {archived ? <ArchiveRestore /> : <Archive />}
              {archived ? 'Unarchive' : 'Archive'}
            </DropdownMenuItem>
            {!isCurrent && (
              <DropdownMenuItem onSelect={actions.restore}>
                <RotateCcw />
                Restore this version
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={actions.remove}
              className="text-destructive focus:bg-destructive/10 focus:text-destructive [&_svg]:text-destructive"
            >
              <Trash2 />
              Delete parcel…
            </DropdownMenuItem>
            <DropdownMenuSeparator />
          </>
        )}
        <DropdownMenuCheckboxItem checked={autoReload} onCheckedChange={onAutoReloadChange}>
          Reload when a new version is published
        </DropdownMenuCheckboxItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
