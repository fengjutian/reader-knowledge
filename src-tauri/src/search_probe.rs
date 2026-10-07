//! 一次性排查脚本：验证 FTS5 对中文与特殊字符的实际行为。
//! 用法：cargo test --lib search_probe -- --nocapture
#[cfg(test)]
mod probe {
    use rusqlite::Connection;

    fn setup(tokenize: &str) -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&format!(
            "CREATE VIRTUAL TABLE notes_fts USING fts5(note_id UNINDEXED,title,content,tokenize='{tokenize}');
             INSERT INTO notes_fts(note_id,title,content) VALUES('h1','置身事内','地方政府的债务问题');"
        ))
        .unwrap();
        c
    }

    fn try_match(c: &Connection, q: &str) -> String {
        match c.query_row(
            "SELECT count(*) FROM notes_fts WHERE notes_fts MATCH ?1",
            rusqlite::params![q],
            |r| r.get::<_, i64>(0),
        ) {
            Ok(count) => format!("{count} hits"),
            Err(error) => format!("ERR {error}"),
        }
    }

    #[test]
    fn probe_tokenizer_split() {
        for tokenize in ["unicode61", "unicode61 remove_diacritics 2", "trigram", "porter unicode61"] {
            let Ok(c) = Connection::open_in_memory().map(|c| {
                c.execute_batch(&format!(
                    "CREATE VIRTUAL TABLE t USING fts5(content,tokenize='{tokenize}');
                     INSERT INTO t(content) VALUES('地方政府的债务问题');"
                ))
                .unwrap();
                c
            }) else { continue };
            let _ = c;
        }
    }

    #[test]
    fn probe_unicode61_queries() {
        let c = setup("unicode61");
        for q in [
            "地方政府",
            "\"地方政府\"",
            "\"地方政府的债务问题\"",
            "\"地方\"",
            "政府",
            "\"债务问题\"",
        ] {
            println!("unicode61 {q:?} => {}", try_match(&c, q));
        }
        // 直接看 tokenizer 把内容切成了什么
        let tokens = c
            .query_row(
                "SELECT content FROM notes_fts WHERE notes_fts MATCH '地方政府'",
                [],
                |r| r.get::<_, String>(0),
            );
        println!("unicode61 词法切分结果: {tokens:?}");
    }

    #[test]
    fn probe_trigram_queries() {
        let c = setup("trigram");
        for q in [
            "地方政府",
            "\"地方政府\"",
            "\"地方\"",
            "\"地方政府\" OR \"政府\"",
        ] {
            println!("trigram {q:?} => {}", try_match(&c, q));
        }
    }

    #[test]
    fn probe_quote_escaping() {
        let c = setup("unicode61");
        for q in [
            "\"\"",
            "\"a\"\"b\"",
            "\"a\"\"\"\"b\"",
            "a\"b",
            "\"NEAR(\"",
            "\"; DROP TABLE notes_fts; --\"",
            "\"*\"",
            "\"^\"",
            "\":\"",
        ] {
            println!("escape {q:?} => {}", try_match(&c, q));
        }
    }
}
