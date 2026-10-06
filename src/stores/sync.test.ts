import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api/tauri";
import { LIBRARY_UPDATED_EVENT, useSyncStore } from "../stores/sync";

const progress = { status: "complete", progress: 100, books: 12, highlights: 30, thoughts: 4, processedBooks: 12, totalBooks: 12 };

describe("sync store revision", () => {
  beforeEach(() => {
    useSyncStore.setState({ status: "idle", progress: 0, books: 0, highlights: 0, thoughts: 0, processedBooks: 0, totalBooks: 0, revision: 0, completedAt: undefined });
  });

  it("成功同步后递增 revision 并广播 library-updated", async () => {
    vi.spyOn(api, "sync").mockResolvedValue(progress);
    const listener = vi.fn();
    window.addEventListener(LIBRARY_UPDATED_EVENT, listener);

    await useSyncStore.getState().run();

    expect(useSyncStore.getState().revision).toBe(1);
    expect(useSyncStore.getState().completedAt).toBeTypeOf("number");
    expect(listener).toHaveBeenCalledTimes(1);
    const detail = (listener.mock.calls[0][0] as CustomEvent).detail;
    expect(detail.source).toBe("weread");
    expect(detail.books).toBe(12);
    expect(detail.highlights).toBe(30);
    expect(detail.completedAt).toBeTypeOf("number");
    window.removeEventListener(LIBRARY_UPDATED_EVENT, listener);
  });

  it("同步失败时不递增 revision，也不广播成功事件", async () => {
    useSyncStore.setState({ revision: 3, books: 99 });
    vi.spyOn(api, "sync").mockRejectedValue(new Error("网络异常"));
    const listener = vi.fn();
    window.addEventListener(LIBRARY_UPDATED_EVENT, listener);

    await useSyncStore.getState().run();

    expect(useSyncStore.getState().revision).toBe(3);
    expect(useSyncStore.getState().status).toBe("failed");
    expect(useSyncStore.getState().message).toBe("网络异常");
    // 失败必须保留上一次成功数据。
    expect(useSyncStore.getState().books).toBe(99);
    expect(listener).not.toHaveBeenCalled();
    window.removeEventListener(LIBRARY_UPDATED_EVENT, listener);
  });

  it("连续两次成功同步使 revision 累加", async () => {
    vi.spyOn(api, "sync").mockResolvedValue(progress);
    await useSyncStore.getState().run();
    await useSyncStore.getState().run();
    expect(useSyncStore.getState().revision).toBe(2);
  });
});
