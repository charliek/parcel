import {
  Download,
  ExternalLink,
  Globe,
  Link2,
  Lock,
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
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { Visibility } from '@/lib/types';

export interface ViewerActions {
  reload: () => void;
  copyLink: () => void;
  contentUrl: string;
  openInNewTab: (event: MouseEvent<HTMLAnchorElement>) => void;
  downloadEntry: () => void;
  share: () => void;
  edit: () => void;
  uploadVersion: () => void;
  toggleVisibility: () => void;
  restore: () => void;
  remove: () => void;
}

/**
 * Overflow menu. On phones it also carries the actions that sit in the top
 * bar on wider screens; owners get the parcel's settings; everyone gets the
 * viewer preferences.
 */
export function ViewerMoreMenu({
  actions,
  isOwner,
  isCurrent,
  visibility,
  autoReload,
  onAutoReloadChange,
}: {
  actions: ViewerActions;
  isOwner: boolean;
  isCurrent: boolean;
  visibility: Visibility;
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
            <DropdownMenuItem onSelect={actions.toggleVisibility}>
              {visibility === 'public' ? <Lock /> : <Globe />}
              {visibility === 'public' ? 'Make private' : 'Make public'}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={actions.edit}>
              <Pencil />
              Edit details…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={actions.uploadVersion}>
              <Upload />
              Upload new version…
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
