import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useLibraryRevision } from "./useLibraryRevision";
import { useSyncStore } from "../stores/sync";

describe("useLibraryRevision", () => {
  beforeEach(() => {
    useSyncStore.setState({ revision: 0 });
  });

  it("挂载时不触发回调", () => {
    // 页面在 mount 时本来就各自 load 一次；注释声称会跳过首次渲染，
    // 但实现里没有 previous，导致进图谱页会多打一轮全库向量 + O(n²) 余弦。
    useSyncStore.setState({ revision: 3 });
    const onUpdated = vi.fn();
    renderHook(() => useLibraryRevision(onUpdated));
    expect(onUpdated).not.toHaveBeenCalled();
  });

  it("同步成功后触发一次回调", async () => {
    const onUpdated = vi.fn();
    renderHook(() => useLibraryRevision(onUpdated));
    expect(onUpdated).not.toHaveBeenCalled();

    act(() => { useSyncStore.setState({ revision: 1 }); });
    expect(onUpdated).toHaveBeenCalledTimes(1);
    expect(onUpdated).toHaveBeenCalledWith(1);

    act(() => { useSyncStore.setState({ revision: 2 }); });
    expect(onUpdated).toHaveBeenCalledTimes(2);
  });

  it("重新挂载同一 revision 不重复触发", () => {
    useSyncStore.setState({ revision: 5 });
    const first = vi.fn();
    const { unmount } = renderHook(() => useLibraryRevision(first));
    expect(first).not.toHaveBeenCalled();
    unmount();

    const second = vi.fn();
    renderHook(() => useLibraryRevision(second));
    expect(second).not.toHaveBeenCalled();
  });
});