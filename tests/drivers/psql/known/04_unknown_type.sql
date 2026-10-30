-- A type name that does not exist is 42704 in PostgreSQL and in docs/sql-subset.md. latticed
-- answers 0A000 because it does not tell an unknown name from an unsupported type.
CREATE TABLE s04_bad (a nosuchtype);
SELECT 'a'::nosuchtype;
