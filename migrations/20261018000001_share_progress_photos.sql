-- Progress photos become shareable with friends, still off until the user opts in
ALTER TABLE friend_sharing ADD COLUMN photos BOOLEAN NOT NULL DEFAULT false;
