import { useEffect, useRef } from "react";
import { useSyncStore } from "../stores/sync";

/**
 * 订阅同步版本号。revision 只在一次成功同步完成后递增，
 * 因此跳过首次渲染可以避免挂载时多余的重复请求——
 * 那些页面在 mount 时本来就各自 load 了一次。
 *
 * @param onUpdated 仅在「本次会话内确实发生过一次新同步」时调用。
 */
export function useLibraryRevision(onUpdated: (revision: number) => void) {
  const revision = useSyncStore(state => state.revision);
  const lastSeen = useRef(revision);
  useEffect(() => {
    // revision 没变说明只是重新挂载，不是新同步，不该再打一轮请求。
    if (revision === lastSeen.current) return;
    lastSeen.current = revision;
    onUpdated(revision);
    // onUpdated 由调用方用 useCallback 稳定化；这里刻意只跟随 revision 触发。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revision]);
}