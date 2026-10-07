PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS books (book_id TEXT PRIMARY KEY,title TEXT NOT NULL,author TEXT,cover TEXT,category TEXT,deep_link TEXT,read_update_time INTEGER,finish_reading INTEGER DEFAULT 0,update_time INTEGER,created_at INTEGER NOT NULL,synced_at INTEGER NOT NULL,is_deleted INTEGER DEFAULT 0,last_seen_sync_id TEXT);
CREATE TABLE IF NOT EXISTS chapters (id INTEGER PRIMARY KEY AUTOINCREMENT,book_id TEXT NOT NULL,chapter_uid INTEGER NOT NULL,chapter_idx INTEGER,title TEXT,created_at INTEGER NOT NULL,UNIQUE(book_id,chapter_uid));
CREATE TABLE IF NOT EXISTS highlights (bookmark_id TEXT PRIMARY KEY,book_id TEXT NOT NULL,chapter_uid INTEGER,chapter_idx INTEGER,chapter_title TEXT,mark_text TEXT NOT NULL,range_json TEXT,color_style TEXT,create_time INTEGER,synced_at INTEGER NOT NULL,is_deleted INTEGER DEFAULT 0,last_seen_sync_id TEXT);
CREATE TABLE IF NOT EXISTS thoughts (review_id TEXT PRIMARY KEY,book_id TEXT NOT NULL,chapter_uid INTEGER,chapter_idx INTEGER,chapter_name TEXT,content TEXT NOT NULL,abstract TEXT,range_json TEXT,create_time INTEGER,synced_at INTEGER NOT NULL,is_deleted INTEGER DEFAULT 0,last_seen_sync_id TEXT);
CREATE TABLE IF NOT EXISTS weread_raw (id INTEGER PRIMARY KEY AUTOINCREMENT,entity_type TEXT NOT NULL,entity_id TEXT NOT NULL,payload TEXT NOT NULL,fetched_at INTEGER NOT NULL,UNIQUE(entity_type,entity_id));
CREATE TABLE IF NOT EXISTS sync_sessions (id TEXT PRIMARY KEY,source TEXT NOT NULL,started_at INTEGER NOT NULL,finished_at INTEGER,status TEXT NOT NULL,books_fetched INTEGER DEFAULT 0,highlights_fetched INTEGER DEFAULT 0,thoughts_fetched INTEGER DEFAULT 0,error_message TEXT);
CREATE TABLE IF NOT EXISTS sync_state (source TEXT PRIMARY KEY,last_synced_at INTEGER,last_successful_session TEXT);
CREATE TABLE IF NOT EXISTS ai_settings (id INTEGER PRIMARY KEY CHECK(id=1),provider TEXT NOT NULL,endpoint TEXT NOT NULL,model TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS relation_analysis_cache (pair_key TEXT PRIMARY KEY,input_hash TEXT NOT NULL,result_json TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS embedding_settings (id INTEGER PRIMARY KEY CHECK(id=1),provider TEXT NOT NULL,endpoint TEXT NOT NULL,model TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS note_embeddings (note_id TEXT PRIMARY KEY,book_id TEXT NOT NULL,content_hash TEXT NOT NULL,model TEXT NOT NULL,vector_json TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_note_embeddings_book ON note_embeddings(book_id);
CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(note_id UNINDEXED,note_type UNINDEXED,book_id UNINDEXED,title,chapter_title,content,tokenize='unicode61');
CREATE TRIGGER IF NOT EXISTS notes_fts_highlights_insert AFTER INSERT ON highlights
WHEN NEW.is_deleted=0 AND EXISTS(SELECT 1 FROM books WHERE book_id=NEW.book_id AND is_deleted=0)
BEGIN
  INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content)
  SELECT NEW.bookmark_id,'highlight',NEW.book_id,title,coalesce(NEW.chapter_title,''),NEW.mark_text
  FROM books WHERE book_id=NEW.book_id AND is_deleted=0;
END;
CREATE TRIGGER IF NOT EXISTS notes_fts_highlights_update AFTER UPDATE OF book_id,chapter_title,mark_text,is_deleted ON highlights
WHEN OLD.book_id IS NOT NEW.book_id OR OLD.chapter_title IS NOT NEW.chapter_title OR OLD.mark_text IS NOT NEW.mark_text OR OLD.is_deleted IS NOT NEW.is_deleted
BEGIN
  DELETE FROM notes_fts WHERE note_id=OLD.bookmark_id AND note_type='highlight';
  INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content)
  SELECT NEW.bookmark_id,'highlight',NEW.book_id,title,coalesce(NEW.chapter_title,''),NEW.mark_text
  FROM books WHERE book_id=NEW.book_id AND NEW.is_deleted=0 AND is_deleted=0;
END;
CREATE TRIGGER IF NOT EXISTS notes_fts_highlights_delete AFTER DELETE ON highlights
BEGIN
  DELETE FROM notes_fts WHERE note_id=OLD.bookmark_id AND note_type='highlight';
END;
CREATE TRIGGER IF NOT EXISTS notes_fts_thoughts_insert AFTER INSERT ON thoughts
WHEN NEW.is_deleted=0 AND EXISTS(SELECT 1 FROM books WHERE book_id=NEW.book_id AND is_deleted=0)
BEGIN
  INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content)
  SELECT NEW.review_id,'thought',NEW.book_id,title,coalesce(NEW.chapter_name,''),NEW.content
  FROM books WHERE book_id=NEW.book_id AND is_deleted=0;
END;
CREATE TRIGGER IF NOT EXISTS notes_fts_thoughts_update AFTER UPDATE OF book_id,chapter_name,content,is_deleted ON thoughts
WHEN OLD.book_id IS NOT NEW.book_id OR OLD.chapter_name IS NOT NEW.chapter_name OR OLD.content IS NOT NEW.content OR OLD.is_deleted IS NOT NEW.is_deleted
BEGIN
  DELETE FROM notes_fts WHERE note_id=OLD.review_id AND note_type='thought';
  INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content)
  SELECT NEW.review_id,'thought',NEW.book_id,title,coalesce(NEW.chapter_name,''),NEW.content
  FROM books WHERE book_id=NEW.book_id AND NEW.is_deleted=0 AND is_deleted=0;
END;
CREATE TRIGGER IF NOT EXISTS notes_fts_thoughts_delete AFTER DELETE ON thoughts
BEGIN
  DELETE FROM notes_fts WHERE note_id=OLD.review_id AND note_type='thought';
END;
CREATE TRIGGER IF NOT EXISTS notes_fts_books_update AFTER UPDATE OF title,is_deleted ON books
WHEN OLD.title IS NOT NEW.title OR OLD.is_deleted IS NOT NEW.is_deleted
BEGIN
  DELETE FROM notes_fts WHERE book_id=OLD.book_id;
  INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content)
  SELECT bookmark_id,'highlight',NEW.book_id,NEW.title,coalesce(chapter_title,''),mark_text
  FROM highlights WHERE book_id=NEW.book_id AND is_deleted=0 AND NEW.is_deleted=0;
  INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content)
  SELECT review_id,'thought',NEW.book_id,NEW.title,coalesce(chapter_name,''),content
  FROM thoughts WHERE book_id=NEW.book_id AND is_deleted=0 AND NEW.is_deleted=0;
END;
CREATE INDEX IF NOT EXISTS idx_books_active_updated ON books(is_deleted,read_update_time DESC);
CREATE INDEX IF NOT EXISTS idx_highlights_book_active ON highlights(book_id,is_deleted);
CREATE INDEX IF NOT EXISTS idx_thoughts_book_active ON thoughts(book_id,is_deleted);
CREATE INDEX IF NOT EXISTS idx_highlights_seen ON highlights(last_seen_sync_id);
CREATE INDEX IF NOT EXISTS idx_thoughts_seen ON thoughts(last_seen_sync_id);
CREATE TABLE IF NOT EXISTS book_metadata_sources (
    book_id TEXT NOT NULL,
    source TEXT NOT NULL,
    source_id TEXT NOT NULL,
    source_url TEXT,
    isbn10 TEXT,
    isbn13 TEXT,
    title TEXT,
    authors_json TEXT,
    publisher TEXT,
    published_date TEXT,
    page_count INTEGER,
    subjects_json TEXT,
    cover_url TEXT,
    description TEXT,
    rating REAL,
    rating_count INTEGER,
    raw_json TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,
    PRIMARY KEY (book_id, source),
    FOREIGN KEY (book_id) REFERENCES books(book_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_book_metadata_sources_source ON book_metadata_sources(source, fetched_at DESC);
-- Backfill installations that synced books before WeRead metadata was persisted.
-- Richer fields are filled on the next sync or an explicit metadata refresh.
INSERT OR IGNORE INTO book_metadata_sources(
    book_id,source,source_id,source_url,title,authors_json,subjects_json,cover_url,raw_json,fetched_at
)
SELECT
    book_id,'weread',book_id,deep_link,title,
    CASE WHEN trim(coalesce(author,''))='' THEN '[]' ELSE json_array(author) END,
    CASE WHEN trim(coalesce(category,''))='' THEN '[]' ELSE json_array(category) END,
    cover,
    json_object('bookId',book_id,'title',title,'author',author,'category',category,'cover',cover,'deepLink',deep_link),
    synced_at
FROM books
WHERE is_deleted=0;
CREATE TABLE IF NOT EXISTS book_metadata_extras (
    book_id TEXT NOT NULL,
    source TEXT NOT NULL,
    author_name TEXT,
    author_avatar TEXT,
    author_url TEXT,
    author_bio TEXT,
    table_of_contents TEXT,
    PRIMARY KEY(book_id,source),
    FOREIGN KEY(book_id,source) REFERENCES book_metadata_sources(book_id,source) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS glossary_terms (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    term TEXT NOT NULL UNIQUE,
    canonical_name TEXT NOT NULL,
    aliases_json TEXT NOT NULL DEFAULT '[]',
    definition TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'manual',
    source_title TEXT,
    source_url TEXT,
    wikipedia_snapshot TEXT,
    status TEXT NOT NULL DEFAULT 'confirmed',
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_glossary_terms_status ON glossary_terms(status,term);
-- 迁移标记。books_fts 整体回填按版本号执行一次，之后交给触发器增量维护；
-- 将来索引结构变了就把版本号 +1，启动时会自动重建。
CREATE TABLE IF NOT EXISTS app_meta (key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS books_fts USING fts5(book_id UNINDEXED,title,author,category,isbn,tokenize='unicode61');
CREATE TRIGGER IF NOT EXISTS books_fts_insert AFTER INSERT ON books
WHEN NEW.is_deleted=0
BEGIN
  INSERT INTO books_fts(book_id,title,author,category,isbn)
  SELECT NEW.book_id,NEW.title,coalesce(NEW.author,''),coalesce(NEW.category,''),
         coalesce(
           (SELECT m.isbn13 FROM book_metadata_sources m WHERE m.book_id=NEW.book_id AND trim(coalesce(m.isbn13,''))<>'' LIMIT 1),
           (SELECT m.isbn10 FROM book_metadata_sources m WHERE m.book_id=NEW.book_id AND trim(coalesce(m.isbn10,''))<>'' LIMIT 1),
           '');
END;
CREATE TRIGGER IF NOT EXISTS books_fts_update AFTER UPDATE OF title,author,category,is_deleted ON books
WHEN OLD.title IS NOT NEW.title OR OLD.author IS NOT NEW.author OR OLD.category IS NOT NEW.category OR OLD.is_deleted IS NOT NEW.is_deleted
BEGIN
  DELETE FROM books_fts WHERE book_id=OLD.book_id;
  INSERT INTO books_fts(book_id,title,author,category,isbn)
  SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.category,''),
         coalesce(
           (SELECT m.isbn13 FROM book_metadata_sources m WHERE m.book_id=b.book_id AND trim(coalesce(m.isbn13,''))<>'' LIMIT 1),
           (SELECT m.isbn10 FROM book_metadata_sources m WHERE m.book_id=b.book_id AND trim(coalesce(m.isbn10,''))<>'' LIMIT 1),
           '')
  FROM books b WHERE b.book_id=NEW.book_id AND b.is_deleted=0;
END;
CREATE TRIGGER IF NOT EXISTS books_fts_delete AFTER DELETE ON books
BEGIN
  DELETE FROM books_fts WHERE book_id=OLD.book_id;
END;
-- ISBN 存在元数据表里，元数据落库后要把 ISBN 同步进书籍索引。
CREATE TRIGGER IF NOT EXISTS books_fts_metadata_insert AFTER INSERT ON book_metadata_sources
BEGIN
  DELETE FROM books_fts WHERE book_id=NEW.book_id;
  INSERT INTO books_fts(book_id,title,author,category,isbn)
  SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.category,''),
         coalesce(
           (SELECT m.isbn13 FROM book_metadata_sources m WHERE m.book_id=b.book_id AND trim(coalesce(m.isbn13,''))<>'' LIMIT 1),
           (SELECT m.isbn10 FROM book_metadata_sources m WHERE m.book_id=b.book_id AND trim(coalesce(m.isbn10,''))<>'' LIMIT 1),
           '')
  FROM books b WHERE b.book_id=NEW.book_id AND b.is_deleted=0;
END;
CREATE TRIGGER IF NOT EXISTS books_fts_metadata_update AFTER UPDATE OF isbn10,isbn13 ON book_metadata_sources
WHEN OLD.isbn10 IS NOT NEW.isbn10 OR OLD.isbn13 IS NOT NEW.isbn13
BEGIN
  DELETE FROM books_fts WHERE book_id=NEW.book_id;
  INSERT INTO books_fts(book_id,title,author,category,isbn)
  SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.category,''),
         coalesce(
           (SELECT m.isbn13 FROM book_metadata_sources m WHERE m.book_id=b.book_id AND trim(coalesce(m.isbn13,''))<>'' LIMIT 1),
           (SELECT m.isbn10 FROM book_metadata_sources m WHERE m.book_id=b.book_id AND trim(coalesce(m.isbn10,''))<>'' LIMIT 1),
           '')
  FROM books b WHERE b.book_id=NEW.book_id AND b.is_deleted=0;
END;
INSERT OR IGNORE INTO app_meta(key,value) VALUES('books_fts_version','0');
DELETE FROM books_fts WHERE (SELECT value FROM app_meta WHERE key='books_fts_version')<>'1';
INSERT INTO books_fts(book_id,title,author,category,isbn)
SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.category,''),
       coalesce(
         (SELECT m.isbn13 FROM book_metadata_sources m WHERE m.book_id=b.book_id AND trim(coalesce(m.isbn13,''))<>'' LIMIT 1),
         (SELECT m.isbn10 FROM book_metadata_sources m WHERE m.book_id=b.book_id AND trim(coalesce(m.isbn10,''))<>'' LIMIT 1),
         '')
FROM books b
WHERE b.is_deleted=0 AND (SELECT value FROM app_meta WHERE key='books_fts_version')<>'1';
UPDATE app_meta SET value='1' WHERE key='books_fts_version' AND value<>'1';
