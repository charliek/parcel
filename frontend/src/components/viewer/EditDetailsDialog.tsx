import { Loader2 } from 'lucide-react';
import { type FormEvent, useState } from 'react';
import { toast } from 'sonner';
import { SlugField } from '@/components/tags/SlugField';
import { TagInput } from '@/components/tags/TagInput';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { keepDialogOpen } from '@/hooks/useCombobox';
import { useUpdateParcel } from '@/hooks/useParcels';
import { useMe } from '@/hooks/useSession';
import { detailsPatch } from '@/lib/details';
import { errorMessage, errorStatus } from '@/lib/http';
import { checkSlugText } from '@/lib/slugs';
import type { ParcelSummary } from '@/lib/types';

/** What the dialog says when the server answers 409 for a slug. */
const SLUG_TAKEN = 'Already used by another of your parcels.';

interface EditDetailsDialogProps {
  parcel: ParcelSummary;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Edit a parcel's title, description, tags and slug. */
export function EditDetailsDialog(props: EditDetailsDialogProps) {
  // Remount the form each time it opens so it starts from the latest values.
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent onEscapeKeyDown={keepDialogOpen}>
        {props.open && <EditDetailsForm {...props} />}
      </DialogContent>
    </Dialog>
  );
}

function EditDetailsForm({ parcel, onOpenChange }: EditDetailsDialogProps) {
  const update = useUpdateParcel(parcel.id);
  const me = useMe();
  const [title, setTitle] = useState(parcel.title);
  const [description, setDescription] = useState(parcel.description);
  // TagInput only ever holds normalized tags, at most MAX_TAGS of them.
  const [tags, setTags] = useState<string[]>(parcel.tags ?? []);
  const [slug, setSlug] = useState(parcel.slug ?? '');
  const [slugError, setSlugError] = useState<string | null>(null);
  const slugCheck = checkSlugText(slug);

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    const trimmed = title.trim();
    if (!trimmed || !slugCheck.ok) return;
    const body = detailsPatch(parcel, {
      title: trimmed,
      description: description.trim(),
      tags,
      slug: slugCheck.slug,
    });
    if (Object.keys(body).length === 0) {
      onOpenChange(false);
      return;
    }
    update.mutate(body, {
      onSuccess: () => {
        toast.success('Details saved');
        onOpenChange(false);
      },
      onError: (err) => {
        if (body.slug !== undefined && errorStatus(err) === 409) setSlugError(SLUG_TAKEN);
        else if (body.slug !== undefined && /slug/i.test(errorMessage(err))) {
          setSlugError(errorMessage(err));
        } else toast.error('Could not save', { description: errorMessage(err) });
      },
    });
  };

  return (
    <form onSubmit={onSubmit} className="grid gap-5">
      <DialogHeader>
        <DialogTitle>Edit details</DialogTitle>
        <DialogDescription>Changes apply to every version of this parcel.</DialogDescription>
      </DialogHeader>
      <div className="space-y-2">
        <Label htmlFor="edit-title">Title</Label>
        <Input
          id="edit-title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          maxLength={200}
          required
          autoFocus
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="edit-description">Description</Label>
        <Textarea
          id="edit-description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          rows={3}
          maxLength={2000}
          placeholder="What is this parcel for?"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="edit-tags">Tags</Label>
        <TagInput id="edit-tags" value={tags} onChange={setTags} disabled={update.isPending} />
      </div>
      <SlugField
        id="edit-slug"
        value={slug}
        onChange={(value) => {
          setSlug(value);
          setSlugError(null);
        }}
        handle={parcel.owner_handle ?? me?.handle}
        serverError={slugError}
        disabled={update.isPending}
        note={
          parcel.slug
            ? 'Links with the old slug keep working until another of your parcels takes it.'
            : undefined
        }
      />
      <DialogFooter>
        <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
          Cancel
        </Button>
        <Button type="submit" disabled={update.isPending || !title.trim() || !slugCheck.ok}>
          {update.isPending && <Loader2 className="animate-spin" />}
          Save
        </Button>
      </DialogFooter>
    </form>
  );
}
