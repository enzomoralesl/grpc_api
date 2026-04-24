ALTER TABLE users
ADD COLUMN IF NOT EXISTS role TEXT NOT NULL DEFAULT 'user';

WITH first_user AS (
    SELECT id
    FROM users
    ORDER BY created_at ASC
    LIMIT 1
)
UPDATE users
SET role = 'admin'
WHERE id IN (SELECT id FROM first_user)
  AND role = 'user';

CREATE INDEX IF NOT EXISTS idx_users_role ON users (role);
