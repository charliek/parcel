/**
 * CLI device-login user codes (`BCDF-GHJK`).
 *
 * People type these by hand, so input is forgiving: case, spaces and the dash
 * are ignored, and the dash is re-inserted after four characters.
 */

/** Characters in a full code, excluding the dash. */
export const USER_CODE_LENGTH = 8;

/** Uppercase and keep only letters and digits. */
function compact(raw: string): string {
  return raw.toUpperCase().replace(/[^A-Z0-9]/g, '');
}

/**
 * Format partial input as the user types: `bcdfg` -> `BCDF-G`. Extra
 * characters beyond a full code are dropped.
 */
export function formatUserCodeInput(raw: string): string {
  const chars = compact(raw).slice(0, USER_CODE_LENGTH);
  return chars.length > 4 ? `${chars.slice(0, 4)}-${chars.slice(4)}` : chars;
}

/**
 * Normalize a complete code (`bcdf ghjk`, `BCDFGHJK`, ` bcdf-ghjk `) to the
 * canonical `BCDF-GHJK`, or `null` when it is not exactly eight letters/digits.
 */
export function normalizeUserCode(raw: string | null | undefined): string | null {
  if (!raw) return null;
  const chars = compact(raw);
  if (chars.length !== USER_CODE_LENGTH) return null;
  return `${chars.slice(0, 4)}-${chars.slice(4)}`;
}
