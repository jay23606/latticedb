-- Errors carry the SQLSTATE PostgreSQL uses for the same situation (psql prints only the code here).
CREATE TABLE s04_t (id INT PRIMARY KEY, name TEXT NOT NULL, code VARCHAR(3), u TEXT UNIQUE);
INSERT INTO s04_t VALUES (1, 'a', 'abc', 'x');
-- syntax
SELECT 1 1;
SELECT FROM FROM s04_t;
-- names
SELECT * FROM s04_missing;
SELECT nocol FROM s04_t;
INSERT INTO s04_missing VALUES (1);
UPDATE s04_missing SET a = 1;
DELETE FROM s04_missing;
DROP TABLE s04_missing;
CREATE TABLE s04_t (id INT);
-- constraints
INSERT INTO s04_t VALUES (1, 'dup', NULL, NULL);
INSERT INTO s04_t VALUES (2, 'dup', NULL, 'x');
INSERT INTO s04_t VALUES (3, NULL, NULL, NULL);
INSERT INTO s04_t (id) VALUES (4);
UPDATE s04_t SET name = NULL WHERE id = 1;
INSERT INTO s04_t VALUES (5, 'long', 'abcd', NULL);
-- evaluation
SELECT 1 / 0;
SELECT 2147483647 + 1;
SELECT 'abc'::int;
SELECT 1 % 0;
-- the statement after each error still works, and nothing above changed the table
SELECT id, name, code, u FROM s04_t ORDER BY id;
DROP TABLE s04_t;
