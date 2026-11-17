use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("凭证读写失败：{0}")]
    Keyring(#[from] keyring::Error),

    #[error("网络请求失败：{0}")]
    Http(#[from] reqwest::Error),

    #[error("接口返回错误：{0}")]
    Api(String),

    #[error("文件读写失败：{0}")]
    Io(#[from] std::io::Error),

    #[error("配置解析失败：{0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
