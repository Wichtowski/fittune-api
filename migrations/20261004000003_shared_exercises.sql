-- Exercises a user creates are shared with everyone from now on. The ones created before were
-- made as private, and their names and notes may say things their owners never meant for
-- others, so they stay visible to their owner (and to admins) only
-- Visibility hangs on this column alone, for catalog exercises too, which are all shared
ALTER TABLE exercises ADD COLUMN shared BOOLEAN NOT NULL DEFAULT true;

UPDATE exercises SET shared = false WHERE owner_id IS NOT NULL;
