import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * KnowledgeGraph3D 的 `data` useMemo 会读取 `showIsolated`，
 * 但依赖数组漏了它：勾选「仅显示有关联」后无关联书籍不会重新出现，
 * 直到切换中心书或选中节点才刷新。
 *
 * 这里用静态检查锁住依赖数组，避免为了断言而把 echarts-gl 整套拉进 jsdom。
 * （jsdom 下 import.meta.url 不是 file://，所以按 cwd 相对定位。）
 */
const source = readFileSync(resolve(process.cwd(), "src/components/graph/KnowledgeGraph3D.tsx"), "utf8");

describe("KnowledgeGraph3D 的 memo 依赖", () => {
  it("计算 data 的 useMemo 依赖里必须包含 showIsolated", () => {
    const memoStart = source.indexOf("const data = useMemo(");
    expect(memoStart, "应能找到 data 的 useMemo").toBeGreaterThan(-1);

    // memo 体内有嵌套的 `});`，不能用 indexOf；直接匹配收尾的 `}, [...]);`
    const tail = /\},\s*\[([^\]]*)\]\);/.exec(source.slice(memoStart));
    expect(tail, "应能找到该 useMemo 的依赖数组").not.toBeNull();
    const deps = tail![1];

    expect(deps).toContain("nodes");
    expect(deps).toContain("showIsolated");
  });

  it("memo 体内确实用到了 showIsolated", () => {
    const body = source.slice(source.indexOf("const data = useMemo("));
    // showIsolated 不是无用 prop：memo 体里真的读了它。
    expect(body).toContain("showIsolated ? nodes");
  });
});