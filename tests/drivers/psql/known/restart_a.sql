-- Run 1 of the restart case: write, then the runner stops latticed with `shutdown`.
CREATE TABLE s07_kept (id INT PRIMARY KEY, name TEXT NOT NULL);
INSERT INTO s07_kept VALUES (1, 'before the restart'), (2, 'also kept');
