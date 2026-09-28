use crate::error::AppError;
pub struct WeReadClient { api_key:String, http:reqwest::Client, base_url:String }
impl WeReadClient {
    pub fn new(api_key:String)->Self{Self{api_key,http:reqwest::Client::new(),base_url:"https://weread.qq.com".into()}}
    pub async fn test(&self)->Result<(),AppError>{self.http.get(format!("{}/web/user",self.base_url)).bearer_auth(&self.api_key).send().await?.error_for_status()?;Ok(())}
}
