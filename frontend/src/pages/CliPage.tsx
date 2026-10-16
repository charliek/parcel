import { Bot, KeyRound, LogIn, Package, Upload } from 'lucide-react';
import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { CodeBlock } from '@/components/common/CodeBlock';
import { AppShell, PageHeader } from '@/components/layout/AppShell';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';

const APT_INSTALL = `sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://apt.stridelabs.ai/pubkey.gpg | \\
  sudo tee /etc/apt/keyrings/apt-charliek.gpg > /dev/null
echo 'deb [signed-by=/etc/apt/keyrings/apt-charliek.gpg] https://apt.stridelabs.ai noble main' | \\
  sudo tee /etc/apt/sources.list.d/apt-charliek.list
sudo apt update
sudo apt install parcel`;

function Step({
  icon: Icon,
  title,
  children,
}: {
  icon: typeof Package;
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="relative flex gap-4">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
        <Icon className="h-4 w-4" aria-hidden="true" />
      </div>
      <div className="min-w-0 flex-1 space-y-3 pb-2">
        <h2 className="pt-1 font-medium text-base">{title}</h2>
        {children}
      </div>
    </section>
  );
}

function Prose({ children }: { children: ReactNode }) {
  return <p className="text-muted-foreground text-sm leading-relaxed">{children}</p>;
}

function Code({ children }: { children: ReactNode }) {
  return <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px]">{children}</code>;
}

/** `/cli`: install the CLI, sign in, publish, and install the agent skill. */
export function CliPage() {
  useDocumentTitle('CLI setup');
  const origin = window.location.origin;

  return (
    <AppShell>
      <PageHeader
        title="CLI setup"
        description="Publish from your terminal, from scripts, and from AI coding agents."
      />
      <div className="space-y-10">
        <Step icon={Package} title="1. Install the CLI">
          <Tabs defaultValue="brew">
            <TabsList>
              <TabsTrigger value="brew">Homebrew</TabsTrigger>
              <TabsTrigger value="apt">apt</TabsTrigger>
              <TabsTrigger value="source">From source</TabsTrigger>
            </TabsList>
            <TabsContent value="brew" className="space-y-2">
              <Prose>macOS and Linux.</Prose>
              <CodeBlock code="brew install charliek/tap/parcel" label="Homebrew install" />
            </TabsContent>
            <TabsContent value="apt" className="space-y-2">
              <Prose>Ubuntu 24.04+ and Debian-based distributions (amd64, arm64).</Prose>
              <CodeBlock code={APT_INSTALL} label="apt install" />
            </TabsContent>
            <TabsContent value="source" className="space-y-2">
              <Prose>
                From a checkout of the Parcel repository, with a Rust toolchain installed.
              </Prose>
              <CodeBlock code="cargo install --path crates/parcel-cli" label="cargo install" />
            </TabsContent>
          </Tabs>
        </Step>

        <Step icon={LogIn} title="2. Sign in">
          <CodeBlock code={`parcel login --server ${origin}`} label="login command" />
          <Prose>
            The CLI shows a short code and opens <Code>{origin}/device</Code>, where you approve the
            login. It works over SSH too: open the link on any device where you're signed in.
          </Prose>
        </Step>

        <Step icon={Upload} title="3. Publish">
          <CodeBlock code="parcel publish index.html" label="publish command" />
          <Prose>
            You get back a private link. Publishing to an existing parcel adds a new version and
            keeps the link. Run <Code>parcel --help</Code> for every command, including{' '}
            <Code>list</Code>, <Code>open</Code>, <Code>pull</Code>, <Code>versions</Code> and{' '}
            <Code>delete</Code>.
          </Prose>
        </Step>

        <Step icon={Bot} title="4. Teach your agent">
          <CodeBlock code="parcel skill install" label="skill install command" />
          <Prose>
            Installs the Parcel skill into <Code>~/.claude/skills/parcel/</Code>, which Claude Code,
            craze and opencode all read. Your agent can then publish the pages it builds and read
            parcels you share with it.
          </Prose>
        </Step>

        <Step icon={KeyRound} title="Scripts and CI">
          <Prose>
            Create a token on the{' '}
            <Link className="text-primary hover:underline" to="/tokens">
              Tokens
            </Link>{' '}
            page and pass it in the environment instead of running <Code>parcel login</Code>:
          </Prose>
          <CodeBlock code="PARCEL_TOKEN=pcl_… parcel publish report.html" label="token example" />
        </Step>
      </div>
    </AppShell>
  );
}
