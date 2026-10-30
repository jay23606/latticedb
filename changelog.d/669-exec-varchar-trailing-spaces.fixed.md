A value longer than a varchar column keeps its first characters when the excess is only spaces, as PostgreSQL does; other overflow still fails with 22001.
