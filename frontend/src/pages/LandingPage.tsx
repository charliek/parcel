import { Bot, History, ShieldCheck } from 'lucide-react';
import { SignInButton } from '@/components/auth/SignInButton';
import { AppShell } from '@/components/layout/AppShell';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';

const FEATURES = [
  {
    icon: ShieldCheck,
    title: 'Sandboxed by design',
    body: 'Pages render on a separate content origin under a strict sandbox, so untrusted HTML can never touch your account.',
  },
  {
    icon: History,
    title: 'Every publish is a version',
    body: 'Update a parcel and the link stays the same. Older versions stay viewable and can be restored.',
  },
  {
    icon: Bot,
    title: 'Built for agents',
    body: 'One CLI command publishes a file or a folder, and a bundled skill teaches Claude Code, craze and opencode to do it for you.',
  },
];

/** `/` for signed-out visitors. */
export function LandingPage() {
  useDocumentTitle(null);
  const origin = window.location.origin;
  return (
    <AppShell wide className="py-12 sm:py-20">
      <section className="mx-auto flex max-w-2xl flex-col items-center text-center">
        <span className="mb-5 rounded-full border bg-muted/50 px-3 py-1 text-muted-foreground text-xs">
          Self-hosted artifacts
        </span>
        <h1 className="text-balance font-semibold text-4xl tracking-tight sm:text-5xl">
          Publish a page. <span className="text-primary">Get a private link.</span>
        </h1>
        <p className="mt-5 max-w-xl text-pretty text-base text-muted-foreground sm:text-lg">
          Parcel hosts the HTML that you and your agents produce — a single file or a whole folder —
          and renders it safely in a sandbox. Private by default, public with one click.
        </p>
        <div className="mt-8 flex flex-wrap justify-center gap-3">
          <SignInButton size="lg" label="Sign in to Parcel" />
        </div>
      </section>

      <section aria-label="Publishing from the command line" className="mx-auto mt-16 max-w-2xl">
        <div className="overflow-hidden rounded-xl border bg-card shadow-sm">
          <div className="flex items-center gap-1.5 border-b bg-muted/40 px-4 py-2.5">
            <span className="h-2.5 w-2.5 rounded-full bg-foreground/15" />
            <span className="h-2.5 w-2.5 rounded-full bg-foreground/15" />
            <span className="h-2.5 w-2.5 rounded-full bg-foreground/15" />
            <span className="ml-2 text-muted-foreground text-xs">terminal</span>
          </div>
          <pre className="overflow-x-auto p-5 font-mono text-[13px] leading-7">
            <code>
              <span className="text-muted-foreground">$ </span>parcel login --server {origin}
              {'\n'}
              <span className="text-muted-foreground">$ </span>parcel publish report.html
              {'\n'}
              <span className="text-success">✓</span> Published{' '}
              <span className="text-primary">{origin}/p/k7m2qx9vhd4n</span>
              {'\n'}
              <span className="text-muted-foreground">$ </span>parcel skill install
              {'\n'}
              <span className="text-success">✓</span> Installed the Parcel skill for Claude Code,
              craze and opencode
            </code>
          </pre>
        </div>
      </section>

      <section className="mx-auto mt-16 grid max-w-4xl gap-4 sm:grid-cols-3">
        {FEATURES.map(({ icon: Icon, title, body }) => (
          <div key={title} className="rounded-xl border bg-card p-5">
            <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <Icon className="h-[18px] w-[18px]" aria-hidden="true" />
            </div>
            <h2 className="font-medium text-sm">{title}</h2>
            <p className="mt-1.5 text-muted-foreground text-sm leading-relaxed">{body}</p>
          </div>
        ))}
      </section>
    </AppShell>
  );
}
