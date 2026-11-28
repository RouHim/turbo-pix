-- The saved-searches feature has been removed. Drop its table; SQLite also
-- removes the table's unique index and sqlite_sequence entry with it.
DROP TABLE IF EXISTS saved_searches;
