import { Check, Copy } from 'lucide-react';
import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Button, type ButtonProps } from '@/components/ui/button';

interface CopyButtonProps extends Omit<ButtonProps, 'onClick' | 'children' | 'value'> {
  value: string;
  /** Optional label rendered next to the icon. */
  label?: string;
  /** Toast shown on success; pass null for none. */
  toastMessage?: string | null;
}

/** Copy text to the clipboard, falling back to a hidden textarea when needed. */
export async function copyText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    return true;
  } catch {
    try {
      const area = document.createElement('textarea');
      area.value = value;
      area.setAttribute('readonly', '');
      area.style.position = 'fixed';
      area.style.opacity = '0';
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand('copy');
      area.remove();
      return ok;
    } catch {
      return false;
    }
  }
}

/** A button that copies `value` and flips to a check briefly. */
export function CopyButton({
  value,
  label,
  toastMessage = 'Copied to clipboard',
  variant = 'outline',
  size,
  ...props
}: CopyButtonProps) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(t);
  }, [copied]);

  const handleCopy = async () => {
    if (await copyText(value)) {
      setCopied(true);
      if (toastMessage) toast.success(toastMessage);
    } else {
      toast.error('Could not copy to the clipboard');
    }
  };

  return (
    <Button
      type="button"
      variant={variant}
      size={size ?? (label ? 'sm' : 'icon-sm')}
      onClick={handleCopy}
      aria-label={label ? undefined : (props['aria-label'] ?? 'Copy')}
      {...props}
    >
      {copied ? <Check className="text-success" /> : <Copy />}
      {label && <span>{copied ? 'Copied' : label}</span>}
    </Button>
  );
}
