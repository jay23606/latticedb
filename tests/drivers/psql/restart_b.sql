-- Run 2 on the same data directory: the table and its rows are still there.
SELECT id, name FROM s07_kept ORDER BY id;
INSERT INTO s07_kept VALUES (3, 'after the restart');
SELECT id FROM s07_kept ORDER BY id;
DROP TABLE s07_kept;
