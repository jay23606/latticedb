-- DDL and single-row and multi-row changes with their command tags.
CREATE TABLE s01_items (id INT PRIMARY KEY, name TEXT NOT NULL, qty BIGINT, ok BOOLEAN);
INSERT INTO s01_items VALUES (1, 'bolt', 10, true);
INSERT INTO s01_items VALUES (2, 'nut', NULL, false), (3, 'washer', 7, NULL);
SELECT * FROM s01_items ORDER BY id;
UPDATE s01_items SET qty = qty + 1 WHERE id = 1;
UPDATE s01_items SET qty = 0 WHERE id > 100;
UPDATE s01_items SET ok = true WHERE qty IS NOT NULL;
SELECT id, qty, ok FROM s01_items ORDER BY id;
DELETE FROM s01_items WHERE id = 2;
DELETE FROM s01_items WHERE id = 2;
SELECT id, name FROM s01_items ORDER BY id;
INSERT INTO s01_items (id, name) VALUES (4, 'pin') RETURNING id, name, qty;
UPDATE s01_items SET qty = 1 WHERE id = 4 RETURNING id, qty;
DELETE FROM s01_items WHERE id = 4 RETURNING name;
DELETE FROM s01_items;
SELECT * FROM s01_items;
DROP TABLE s01_items;
DROP TABLE s01_items;
