-- Rebuild a parcel's search vector only when its document changed.
--
-- `parcels_search_b_vec` (0006) fires for every UPDATE that *names* title,
-- description, tags, search_text or current_version, changed or not: a
-- visibility-only PATCH or a new version with no new metadata recomputed up
-- to 128 KiB of page text for nothing. Now an UPDATE whose four inputs are
-- all what they were keeps the vector it has. `NEW` already reflects
-- `parcels_a_version_changed`, which runs first (name order): a version
-- bump that clears stored page text changes search_text and still
-- recomputes; one that finds the text already NULL changes nothing in the
-- document, and keeps the vector.
--
-- Lost escape hatch: "touch the row" no longer rebuilds a vector. A future
-- change to `parcel_search_document` or to the text-search configuration
-- must recompute existing rows explicitly, as 0006 did:
--   UPDATE parcels SET search_vec =
--       parcel_search_document(title, description, tags, search_text);
--
-- Additive and rollout-safe: the trigger definition is unchanged, and an
-- older revision's writes run through the same function.

CREATE OR REPLACE FUNCTION parcels_search_vec() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE'
        AND NEW.title IS NOT DISTINCT FROM OLD.title
        AND NEW.description IS NOT DISTINCT FROM OLD.description
        AND NEW.tags IS NOT DISTINCT FROM OLD.tags
        AND NEW.search_text IS NOT DISTINCT FROM OLD.search_text
    THEN
        RETURN NEW;
    END IF;
    NEW.search_vec := parcel_search_document(NEW.title, NEW.description, NEW.tags, NEW.search_text);
    RETURN NEW;
END
$$;
