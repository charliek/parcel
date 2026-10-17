-- v0.4: comments written through the MCP server are recorded as 'mcp', so the
-- viewer can say "via MCP" (an assistant acting as the signed-in person)
-- rather than passing them off as the CLI's or the person's own.
ALTER TABLE comments DROP CONSTRAINT comments_via_check;
ALTER TABLE comments
    ADD CONSTRAINT comments_via_check CHECK (via IN ('web', 'cli', 'mcp'));
