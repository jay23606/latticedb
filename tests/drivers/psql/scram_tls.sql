-- Run by scram_tls.sh over SCRAM-SHA-256 and TLS (sslmode=verify-full).
CREATE TABLE s08_secure (id INT PRIMARY KEY, note TEXT);
INSERT INTO s08_secure VALUES (1, 'over tls');
SELECT id, note FROM s08_secure;
SELECT * FROM s08_missing;
DROP TABLE s08_secure;
