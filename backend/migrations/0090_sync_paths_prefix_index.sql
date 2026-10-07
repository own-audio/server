-- Path claims test "is anything below this folder?" with a LIKE 'prefix/%'
-- query; text_pattern_ops makes that an index range scan instead of a scan of
-- every path the owner has. Before it, a first library scan of 10,000 files
-- took one query per file over all earlier files (docs/CAPACITY.md).
CREATE INDEX sync_paths_owner_prefix_idx ON sync_paths (user_id, lower(path) text_pattern_ops);
