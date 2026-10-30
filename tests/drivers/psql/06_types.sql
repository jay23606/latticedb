-- Column types in text format: round trips, ranges, casts and boolean NULL logic.
CREATE TABLE s06_t (i SMALLINT, b BIGINT, f DOUBLE PRECISION, t TEXT, v VARCHAR(4), y BYTEA, d DATE, u UUID, ts TIMESTAMPTZ, ok BOOLEAN);
INSERT INTO s06_t VALUES (32767, 9223372036854775807, 0.1, 'héllo', 'abcd', '\x00ff41', '2024-02-29', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '2024-01-02 03:04:05+02', true);
SELECT * FROM s06_t;
INSERT INTO s06_t (i) VALUES (32768);
INSERT INTO s06_t (b) VALUES (9223372036854775808);
SELECT 9223372036854775807 + 1;
SELECT '2024-02-30'::date;
SELECT 'zz'::uuid;
SELECT date '2024-03-01' < date '2024-04-01' AS earlier;
SELECT 0.1::double precision + 0.2::double precision AS s;
SELECT 1.5::numeric + 1.25 AS n;
SELECT true AND NULL AS a, false AND NULL AS b, true OR NULL AS c, NOT NULL::boolean AS d;
SELECT CAST('12' AS int) + 1 AS twelve_plus_one;
SELECT i, b FROM s06_t;
DROP TABLE s06_t;
