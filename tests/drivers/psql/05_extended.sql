-- Extended query protocol: psql \bind sends Parse, Bind (text parameters), Describe and Execute.
CREATE TABLE s05_t (id INT PRIMARY KEY, name TEXT, flag BOOLEAN, big BIGINT);
INSERT INTO s05_t VALUES ($1, $2, $3, $4) \bind 1 alpha true 9000000000 \g
INSERT INTO s05_t VALUES ($1, nullif($2, ''), $3, $4) \bind 2 '' false 0 \g
SELECT id, name, flag, big FROM s05_t ORDER BY id \bind \g
SELECT id, name FROM s05_t WHERE id = $1 \bind 1 \g
SELECT id FROM s05_t WHERE name = $1 \bind alpha \g
SELECT id FROM s05_t WHERE name = $1 \bind nosuchname \g
SELECT $1::int + $2::int AS sum \bind 40 2 \g
SELECT $1::text AS t, $2::boolean AS b \bind 'it''s' f \g
UPDATE s05_t SET big = $2 WHERE id = $1 \bind 1 5 \g
UPDATE s05_t SET big = $2 WHERE id = $1 \bind 99 5 \g
SELECT big FROM s05_t WHERE id = $1 \bind 1 \g
DELETE FROM s05_t WHERE id = $1 \bind 2 \g
DELETE FROM s05_t WHERE id = $1 \bind 2 \g
INSERT INTO s05_t VALUES ($1, $2, $3, $4) RETURNING id, name \bind 3 gamma true 3 \g
-- Errors in the extended protocol, and the connection afterwards.
INSERT INTO s05_t VALUES ($1, $2, $3, $4) \bind 1 dup true 1 \g
SELECT $1::int AS n \bind notanumber \g
SELECT $1::int / $2::int AS q \bind 1 0 \g
SELECT * FROM s05_missing WHERE id = $1 \bind 1 \g
SELECT id FROM s05_t ORDER BY id \bind \g
-- A block with an extended-protocol error is aborted, as with the simple protocol.
BEGIN;
INSERT INTO s05_t VALUES ($1, $2, $3, $4) \bind 10 ten true 10 \g
INSERT INTO s05_t VALUES ($1, $2, $3, $4) \bind 10 again true 10 \g
SELECT id FROM s05_t WHERE id = $1 \bind 10 \g
COMMIT;
SELECT id FROM s05_t ORDER BY id \bind \g
DROP TABLE s05_t;
