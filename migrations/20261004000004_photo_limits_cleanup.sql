ALTER TABLE progress_photos ADD COLUMN stored_bytes BIGINT;
-- Earlier rows did not record thumbnail sizes, so reserve a conservative allowance
UPDATE progress_photos SET stored_bytes = bytes::bigint * 2;
ALTER TABLE progress_photos ALTER COLUMN stored_bytes SET NOT NULL;
ALTER TABLE progress_photos ADD CHECK (stored_bytes >= bytes);

CREATE TABLE photo_cleanup (
    storage_key TEXT PRIMARY KEY,
    available_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX photo_cleanup_available_idx ON photo_cleanup (available_at);

CREATE FUNCTION queue_deleted_photo() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO photo_cleanup (storage_key) VALUES (OLD.storage_key)
    ON CONFLICT (storage_key) DO UPDATE SET available_at = now();
    RETURN OLD;
END;
$$;

-- Account cascades and individual photo deletes enqueue storage cleanup in the same transaction
CREATE TRIGGER progress_photo_cleanup AFTER DELETE ON progress_photos
FOR EACH ROW EXECUTE FUNCTION queue_deleted_photo();
