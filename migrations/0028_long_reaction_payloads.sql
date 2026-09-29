-- A valid 64-character custom emoji shortcode is wrapped as `:shortcode:` in
-- NIP-25 reaction content. Preserve the wrapper in the reaction projection.
ALTER TABLE reactions ALTER COLUMN emoji TYPE VARCHAR(66);
