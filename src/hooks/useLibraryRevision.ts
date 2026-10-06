import { useEffect } from "react";
import { useSyncStore } from "../stores/sync";

/**
 * 订阅同步版本号。revision 只在一次成功同步完成后递增，
 * 因此用 `previous` 跳过首次渲染可以避免挂载时多余的重复请求。
 */
export function useLibraryRevision(onUpdated: (revision: number) => void) {
  const revision = useSyncStore(state => state.revision);
  useEffect(() => {
    if (revision > 0) onUpdated(revision);
    // onUpdated 由调用方用 useCallback 稳定化；这里刻意只跟随 revision 触发。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revision]);
}
