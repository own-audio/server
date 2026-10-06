-- Compositing the title and author onto generated cover art is now opt-in.
-- It was unconditional, which looks wrong on art that already reads as a
-- finished cover, and the library shows the title under the artwork anyway.
ALTER TABLE generation_jobs
    ADD COLUMN cover_show_title BOOLEAN NOT NULL DEFAULT false;
