-- Page titles.
--
-- version_files.page_title  the `<title>` or first `# heading` of a page (an
--                           .html/.htm/.md/.markdown file), read at publish,
--                           ≤ 200 characters, control characters stripped;
--                           NULL for every other file, for a page with no
--                           title, and for rows written before this
--                           migration (the web app then shows the path)
--
-- Additive and rollout-safe: nullable, so the previous revision keeps
-- inserting file rows during a rollover. A merge it publishes copies its
-- kept files without their titles, which only means those pages show their
-- paths until the next publish.

ALTER TABLE version_files ADD COLUMN page_title TEXT;
