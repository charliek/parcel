/**
 * Parcel ids and version route params (`crates/parcel-types/src/ids.rs`).
 *
 * Checking the shape first means a mistyped link shows "not found" without a
 * request, and never triggers a sign-in bounce.
 */

/** The id alphabet: lowercase letters and digits minus the easily confused 0, o, 1, l. */
export const ID_ALPHABET = 'abcdefghijkmnpqrstuvwxyz23456789';
export const ID_LEN = 12;

const ID_RE = new RegExp(`^[${ID_ALPHABET}]{${ID_LEN}}$`);

/** Whether `s` is shaped like a parcel id. Says nothing about existence. */
export function isValidParcelId(s: string | null | undefined): boolean {
  return typeof s === 'string' && ID_RE.test(s);
}

/**
 * Parse the `:version` route param: a positive integer, `null` when absent,
 * or `'invalid'`.
 */
export function parseVersionParam(raw: string | null | undefined): number | null | 'invalid' {
  if (raw === undefined || raw === null) return null;
  if (!/^[1-9]\d{0,8}$/.test(raw)) return 'invalid';
  return Number(raw);
}
