use crate::{error::AppError, models::ChatMessage};
use std::{future::Future, pin::Pin};
pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = Result<String, AppError>> + Send + 'a>>;
pub trait AiProvider: Send + Sync {
    fn id(&self) -> &str;
    fn chat<'a>(&'a self, messages: &'a [ChatMessage]) -> ProviderFuture<'a>;
}
