-- A moved or renamed file keeps its size and modification time; the scanner
-- finds its previous row by them (library_folders::adopt_moved).
CREATE INDEX library_files_size_idx ON library_files (folder_id, size_bytes, mtime_secs);
