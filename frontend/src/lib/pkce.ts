/**
 * PKCE (RFC 7636) S256 helpers using the Web Crypto API.
 *
 * The verifier is a high-entropy random string; the challenge is the unpadded
 * base64url SHA-256 of the verifier. State is a separate random value for CSRF
 * protection on the OAuth callback.
 */

const UNRESERVED = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~';

function randomString(length: number): string {
  // Rejection sampling keeps the distribution uniform over the 66 symbols.
  const limit = 256 - (256 % UNRESERVED.length);
  let out = '';
  while (out.length < length) {
    const bytes = new Uint8Array(length * 2);
    crypto.getRandomValues(bytes);
    for (const byte of bytes) {
      if (byte < limit) out += UNRESERVED[byte % UNRESERVED.length];
      if (out.length === length) break;
    }
  }
  return out;
}

function base64UrlEncode(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** Generate a PKCE code verifier (64 chars from the unreserved set, within 43-128). */
export function generateVerifier(): string {
  return randomString(64);
}

/** Compute the S256 code challenge for a verifier. */
export async function challengeFromVerifier(verifier: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier));
  return base64UrlEncode(digest);
}

/** Generate an OAuth `state` value (CSRF protection). */
export function generateState(): string {
  return randomString(32);
}
