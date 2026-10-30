-- PostgreSQL answers COMMIT and ROLLBACK outside a block, and a second BEGIN inside one, with a
-- warning (25P01, 25001) and the usual command tag. latticed sends the tag only.
COMMIT;
ROLLBACK;
BEGIN;
BEGIN;
ROLLBACK;
