-- Keep the raw capability token so an authenticated owner or administrator can
-- display and copy the share URL. Existing shares remain NULL because their
-- one-way token hashes cannot be reversed.
ALTER TABLE shares ADD COLUMN display_token TEXT;
