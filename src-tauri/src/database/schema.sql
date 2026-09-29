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
