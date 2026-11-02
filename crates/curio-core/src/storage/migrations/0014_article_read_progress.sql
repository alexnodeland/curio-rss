-- Per-article reading position (#153): how far through its body the reader
-- last was, in thousandths of the body's scroll extent (0 = the top, 1000 =
-- the end). It lives beside the flags in the article_state projection, so a
-- position save never touches the FTS-indexed articles table. DB-local: no
-- contract event narrates it.
ALTER TABLE article_state ADD COLUMN read_permille INTEGER NOT NULL DEFAULT 0
    CHECK (read_permille BETWEEN 0 AND 1000);
