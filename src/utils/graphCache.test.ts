import { describe, expect, it } from "vitest";
import { GRAPH_ALGORITHM_VERSION, LOCAL_CACHE_PREFIX, SEMANTIC_CACHE_PREFIX, graphCacheKey, semanticCacheKey } from "./graphCache";

const book = (id: string, extra: Partial<{ updatedAt: string; highlightCount: number; thoughtCount: number }> = {}) => ({
  id, updatedAt: extra.updatedAt ?? "2026-01-01", highlightCount: extra.highlightCount ?? 1, thoughtCount: extra.thoughtCount ?? 0,
});
const note = (id: string, extra: Partial<{ bookId: string; chapter: string; content: string; type: string; createdAt: string }> = {}) => ({
  id, bookId: extra.bookId ?? "b1", chapter: extra.chapter ?? "第一章", content: extra.content ?? "内容", type: extra.type ?? "highlight", createdAt: extra.createdAt ?? "2026-01-01",
});

describe("graphCacheKey", () => {
  it("包含算法版本命名空间", () => {
    const key = graphCacheKey({ books: [book("b1")], notes: [note("n1")] });
    expect(key.startsWith(`${LOCAL_CACHE_PREFIX}:`)).toBe(true);
    expect(LOCAL_CACHE_PREFIX).toContain(GRAPH_ALGORITHM_VERSION);
  });

  it("书籍 ID 或笔记数量变化时 key 改变", () => {
    const base = graphCacheKey({ books: [book("b1")], notes: [note("n1")] });
    expect(graphCacheKey({ books: [book("b1"), book("b2")], notes: [note("n1")] })).not.toBe(base);
    expect(graphCacheKey({ books: [book("b1")], notes: [note("n1"), note("n2")] })).not.toBe(base);
  });

  it("书籍更新时间或笔记数量变化时 key 改变", () => {
    const base = graphCacheKey({ books: [book("b1")], notes: [note("n1")] });
    expect(graphCacheKey({ books: [book("b1", { updatedAt: "2026-02-02" })], notes: [note("n1")] })).not.toBe(base);
    expect(graphCacheKey({ books: [book("b1", { highlightCount: 9 })], notes: [note("n1")] })).not.toBe(base);
  });

  it("笔记内容变化时 key 改变（包含内容摘要）", () => {
    const base = graphCacheKey({ books: [book("b1")], notes: [note("n1")] });
    expect(graphCacheKey({ books: [book("b1")], notes: [note("n1", { content: "改过的内容" })] })).not.toBe(base);
  });

  it("笔记 ID 变化时 key 改变", () => {
    const base = graphCacheKey({ books: [book("b1")], notes: [note("n1")] });
    expect(graphCacheKey({ books: [book("b1")], notes: [note("n2")] })).not.toBe(base);
  });

  it("相同输入产生相同 key", () => {
    const input = { books: [book("b1"), book("b2")], notes: [note("n1"), note("n2")] };
    expect(graphCacheKey(input)).toBe(graphCacheKey(input));
  });
});

describe("semanticCacheKey", () => {
  const input = { books: [book("b1")], notes: [note("n1")] };

  it("Embedding 模型名变化时 key 改变", () => {
    const a = semanticCacheKey(input, "BAAI/bge-small-zh-v1.5", ["weread", "douban"]);
    const b = semanticCacheKey(input, "other-model", ["weread", "douban"]);
    expect(a).not.toBe(b);
    expect(a.startsWith(`${SEMANTIC_CACHE_PREFIX}:`)).toBe(true);
  });

  it("元数据来源集合变化时 key 改变", () => {
    const a = semanticCacheKey(input, "m", ["weread"]);
    const b = semanticCacheKey(input, "m", ["weread", "douban"]);
    expect(a).not.toBe(b);
  });

  it("元数据来源顺序不影响 key", () => {
    expect(semanticCacheKey(input, "m", ["weread", "douban"])).toBe(semanticCacheKey(input, "m", ["douban", "weread"]));
  });

  it("本地关系 key 变化会传导到语义 key", () => {
    const base = semanticCacheKey(input, "m", ["weread"]);
    const changed = semanticCacheKey({ books: [book("b1", { highlightCount: 5 })], notes: [note("n1")] }, "m", ["weread"]);
    expect(changed).not.toBe(base);
  });
});
