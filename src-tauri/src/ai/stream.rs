//! 活动中的流式请求登记表。
//!
//! `ask_ai_stream` 会立刻返回，真正的网络读取在独立任务里跑，因此「取消」
//! 只能通过这张表找到对应的取消标记。前端拿到的 `requestId` 就是这里的键。

use crate::error::AppError;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct StreamRegistry {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl StreamRegistry {
    /// 活动请求数量。排查「后台还有没有在跑的生成」时用。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active_count(&self) -> usize {
        self.active.lock().map(|active| active.len()).unwrap_or(0)
    }

    /// 登记一个新的活动请求。`requestId` 重复说明前端状态串了，直接拒绝。
    pub fn register(&self, request_id: &str) -> Result<Arc<AtomicBool>, AppError> {
        let mut active = self.lock()?;
        if active.contains_key(request_id) {
            return Err(AppError::Message("该请求已在生成中".into()));
        }
        let token = Arc::new(AtomicBool::new(false));
        active.insert(request_id.to_owned(), Arc::clone(&token));
        Ok(token)
    }

    /// 取消指定请求。返回是否确实命中了一个活动请求。
    pub fn cancel(&self, request_id: &str) -> bool {
        let Ok(active) = self.active.lock() else { return false };
        match active.get(request_id) {
            Some(token) => {
                token.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn finish(&self, request_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(request_id);
        }
    }

    /// 强制结束所有活动请求。窗口关闭时用来释放网络连接。
    pub fn cancel_all(&self) -> usize {
        let Ok(active) = self.active.lock() else { return 0 };
        let count = active.len();
        for token in active.values() {
            token.store(true, Ordering::Relaxed);
        }
        count
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>, AppError> {
        self.active
            .lock()
            .map_err(|_| AppError::Message("流式请求登记表已损坏".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 登记后可以取消并正确命中() {
        let registry = StreamRegistry::default();
        let token = registry.register("req-1").unwrap();
        assert!(!token.load(Ordering::Relaxed));
        assert_eq!(registry.active_count(), 1);
        assert!(registry.cancel("req-1"));
        assert!(token.load(Ordering::Relaxed));
        assert!(!registry.cancel("req-2"));
    }

    #[test]
    fn 重复的请求标识会被拒绝() {
        let registry = StreamRegistry::default();
        registry.register("req-1").unwrap();
        assert!(registry.register("req-1").is_err());
        registry.finish("req-1");
        assert!(registry.register("req-1").is_ok());
    }

    #[test]
    fn 结束请求后不再占用标识() {
        let registry = StreamRegistry::default();
        registry.register("req-1").unwrap();
        registry.finish("req-1");
        assert_eq!(registry.active_count(), 0);
        assert!(!registry.cancel("req-1"));
    }

    #[test]
    fn 可以一次取消全部活动请求() {
        let registry = StreamRegistry::default();
        let first = registry.register("a").unwrap();
        let second = registry.register("b").unwrap();
        assert_eq!(registry.cancel_all(), 2);
        assert!(first.load(Ordering::Relaxed));
        assert!(second.load(Ordering::Relaxed));
    }
}
