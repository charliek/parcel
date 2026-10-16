import { ArrowLeft, Loader2, Upload } from 'lucide-react';
import { type FormEvent, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { AppShell } from '@/components/layout/AppShell';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { NativeSelect } from '@/components/ui/select';
import { Textarea } from '@/components/ui/textarea';
import { FileDropZone } from '@/components/upload/FileDropZone';
import { FileSelectionList } from '@/components/upload/FileSelectionList';
import { VisibilityPicker } from '@/components/upload/VisibilityPicker';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useFileSelection } from '@/hooks/useFileSelection';
import { invalidateParcel } from '@/hooks/useParcels';
import { api } from '@/lib/api';
import { errorMessage } from '@/lib/http';
import { queryClient } from '@/lib/query-client';
import type { Visibility } from '@/lib/types';
import { buildPublishForm } from '@/lib/upload';
import { useAllowPublic } from '@/stores/server-info.store';

function Section({
  step,
  title,
  description,
  children,
}: {
  step: number;
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-4" aria-labelledby={`step-${step}`}>
      <div className="flex items-baseline gap-3">
        <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-muted font-medium text-muted-foreground text-xs">
          {step}
        </span>
        <div>
          <h2 id={`step-${step}`} className="font-medium text-base">
            {title}
          </h2>
          {description && <p className="text-muted-foreground text-sm">{description}</p>}
        </div>
      </div>
      <div className="sm:pl-9">{children}</div>
    </section>
  );
}

/** `/new`: create a parcel by uploading a file, several files or a folder. */
export function NewParcelPage() {
  useDocumentTitle('New parcel');
  const navigate = useNavigate();
  const selection = useFileSelection({ requireEntry: true });
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [visibility, setVisibility] = useState<Visibility>('private');
  const allowPublic = useAllowPublic();
  const [label, setLabel] = useState('');
  const [progress, setProgress] = useState<number | null>(null);
  const submitting = progress !== null;

  const onSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!selection.check.ok || submitting) return;
    setProgress(0);
    try {
      const form = buildPublishForm(
        {
          title,
          description,
          visibility,
          entry: selection.entry ?? undefined,
          label,
        },
        selection.files,
      );
      const result = await api.createParcel(form, setProgress);
      await invalidateParcel(queryClient, result.parcel.id);
      toast.success('Parcel published', { description: result.parcel.title });
      navigate(`/p/${result.parcel.id}`);
    } catch (err) {
      setProgress(null);
      toast.error('Publish failed', { description: errorMessage(err) });
    }
  };

  const multipleCandidates = selection.candidates.length > 1;

  return (
    <AppShell>
      <Link
        to="/"
        className="mb-6 inline-flex items-center gap-1.5 rounded text-muted-foreground text-sm hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ArrowLeft className="h-4 w-4" aria-hidden="true" />
        My parcels
      </Link>
      <div className="mb-8 space-y-1">
        <h1 className="font-semibold text-2xl tracking-tight">New parcel</h1>
        <p className="text-muted-foreground text-sm">
          Upload a page and anything it references. Relative links keep working because folder
          structure is preserved.
        </p>
      </div>

      <form onSubmit={onSubmit} className="space-y-10">
        <Section step={1} title="Files">
          <div className="space-y-4">
            <FileDropZone
              onItems={selection.add}
              compact={selection.files.length > 0}
              disabled={submitting}
            />
            <FileSelectionList selection={selection} disabled={submitting} />
            {multipleCandidates && (
              <div className="space-y-2">
                <Label htmlFor="entry">Opens on</Label>
                <NativeSelect
                  id="entry"
                  value={selection.chosenEntry ?? ''}
                  onChange={(e) => selection.setEntryChoice(e.target.value || null)}
                  disabled={submitting}
                >
                  <option value="">
                    {selection.entry ? `Automatic (${selection.entry})` : 'Choose a page…'}
                  </option>
                  {selection.candidates.map((path) => (
                    <option key={path} value={path}>
                      {path}
                    </option>
                  ))}
                </NativeSelect>
              </div>
            )}
          </div>
        </Section>

        <Section step={2} title="Details" description="All optional. You can change them later.">
          <div className="space-y-5">
            <div className="space-y-2">
              <Label htmlFor="title">Title</Label>
              <Input
                id="title"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Defaults to the page's <title> or first heading"
                maxLength={200}
                disabled={submitting}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="What is this parcel for?"
                rows={3}
                maxLength={2000}
                disabled={submitting}
              />
            </div>
            <div className="space-y-2">
              <span className="font-medium text-sm" aria-hidden="true">
                Who can open it
              </span>
              <VisibilityPicker
                legend="Who can open it"
                value={visibility}
                onChange={setVisibility}
                allowPublic={allowPublic}
                disabled={submitting}
              />
              <p className="text-muted-foreground text-xs">
                You can share it with people by email once it’s published.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="label">Version note</Label>
              <Input
                id="label"
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder="e.g. First draft"
                maxLength={60}
                disabled={submitting}
              />
            </div>
          </div>
        </Section>

        <div className="flex flex-col-reverse items-stretch gap-3 border-t pt-6 sm:flex-row sm:items-center sm:justify-end">
          {submitting && (
            <span className="text-muted-foreground text-sm sm:mr-auto" aria-live="polite">
              {progress !== null && progress < 1
                ? `Uploading… ${Math.round(progress * 100)}%`
                : 'Processing…'}
            </span>
          )}
          <Button type="button" variant="ghost" onClick={() => navigate('/')} disabled={submitting}>
            Cancel
          </Button>
          <Button type="submit" disabled={!selection.check.ok || submitting}>
            {submitting ? <Loader2 className="animate-spin" /> : <Upload />}
            Publish parcel
          </Button>
        </div>
      </form>
    </AppShell>
  );
}
