#[derive(Debug, Clone)]
pub struct WebDavEndpoint {
    pub base_url: String,
    pub root_path: String,
    pub username: Option<String>,
    pub password_ref: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebDavStat {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub etag: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebDavListEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub etag: Option<String>,
}
