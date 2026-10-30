-- Transaction blocks: commit, rollback, the aborted state after an error, and misuse.
CREATE TABLE s03_acct (id INT PRIMARY KEY, bal INT NOT NULL);
INSERT INTO s03_acct VALUES (1, 100), (2, 50);
BEGIN;
UPDATE s03_acct SET bal = bal - 30 WHERE id = 1;
UPDATE s03_acct SET bal = bal + 30 WHERE id = 2;
SELECT id, bal FROM s03_acct ORDER BY id;
COMMIT;
SELECT id, bal FROM s03_acct ORDER BY id;
BEGIN;
DELETE FROM s03_acct WHERE id = 2;
INSERT INTO s03_acct VALUES (3, 1);
SELECT id FROM s03_acct ORDER BY id;
ROLLBACK;
SELECT id FROM s03_acct ORDER BY id;
-- An error inside a block aborts it: later statements are refused with 25P02 and COMMIT rolls back.
BEGIN;
INSERT INTO s03_acct VALUES (4, 1);
INSERT INTO s03_acct VALUES (1, 1);
SELECT 1;
INSERT INTO s03_acct VALUES (5, 1);
COMMIT;
SELECT id FROM s03_acct ORDER BY id;
-- The same with an explicit ROLLBACK, and a division by zero as the error.
BEGIN;
SELECT 1 / 0;
SELECT id FROM s03_acct;
ROLLBACK;
-- The connection is usable again afterwards.
SELECT id FROM s03_acct WHERE id = 1;
-- A failed statement outside a block changes nothing.
INSERT INTO s03_acct VALUES (6, NULL);
SELECT id FROM s03_acct ORDER BY id;
DROP TABLE s03_acct;
