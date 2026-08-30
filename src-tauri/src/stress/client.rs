use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

/// 共享 HTTP 客户端（keep-alive，全请求复用）
pub struct StressClient {
    pub client: Client,
    pub base_url: String,
}

impl StressClient {
    pub fn new(base_url: &str, auth_token: &str, auth_headers_json: &str) -> Result<Self, String> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Accept", "application/json, application/octet-stream".parse().unwrap());

        if !auth_token.is_empty() {
            headers.insert("Authorization", format!("Bearer {}", auth_token).parse().unwrap());
        }
        if !auth_headers_json.trim().is_empty() {
            let parsed: HashMap<String, String> = serde_json::from_str(auth_headers_json)
                .map_err(|e| format!("AUTH_HEADERS 不是合法 JSON: {}", e))?;
            for (k, v) in parsed {
                if let Ok(name) = k.parse::<reqwest::header::HeaderName>() {
                    headers.insert(name, v.parse().unwrap());
                }
            }
        }

        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// 上传文件（分拣控件同款 POST /doc/file/upload）
    pub async fn upload(&self, file_path: &str) -> Result<String, String> {
        let name = file_path.rsplit('/').next().unwrap_or(file_path);
        let bytes = tokio::fs::read(file_path)
            .await
            .map_err(|e| format!("读文件失败 {}: {}", name, e))?;
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.to_string())
            .mime_str("application/octet-stream")
            .unwrap();
        let form = reqwest::multipart::Form::new().part("file", part);

        let resp = self
            .client
            .post(format!("{}/doc/file/upload", self.base_url))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("上传 {} 失败: {}", name, e))?;

        let body: Value = resp.json().await.map_err(|e| format!("上传 {} 响应解析失败: {}", name, e))?;
        let data = &body["data"];
        let file_id = data["objectId"]
            .as_str()
            .or_else(|| data["fileId"].as_str())
            .or_else(|| data["id"].as_str())
            .ok_or_else(|| format!("上传 {} 响应缺少 fileId: {}", name, body.to_string().chars().take(200).collect::<String>()))?;
        Ok(file_id.to_string())
    }

    /// 预登记（POST /doc/prepare）
    pub async fn prepare(&self, file_id: &str, file_name: &str) -> Result<(), String> {
        let body = serde_json::json!({ "files": [{ "fileId": file_id, "fileName": file_name }] });
        let resp = self
            .client
            .post(format!("{}/doc/prepare", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("prepare 失败: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("prepare HTTP {}", resp.status()));
        }
        Ok(())
    }

    /// 查询 state（POST /doc/state）
    pub async fn state(&self, file_id: &str, file_name: &str, preview_type: &str) -> Result<Value, String> {
        let body = serde_json::json!({
            "file": { "fileId": file_id, "fileName": file_name },
            "previewType": preview_type
        });
        let resp = self
            .client
            .post(format!("{}/doc/state", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("state {} 失败: {}", file_name, e))?;
        let json: Value = resp.json().await.map_err(|e| format!("state 响应解析失败: {}", e))?;
        Ok(json["data"].clone())
    }

    /// range 拉取（POST /doc/range），返回字节数
    pub async fn range(&self, file_id: &str, file_name: &str, begin: u64, end: u64) -> Result<u64, String> {
        let body = serde_json::json!({
            "file": { "fileId": file_id, "fileName": file_name },
            "range": { "begin": begin, "end": end }
        });
        let resp = self
            .client
            .post(format!("{}/doc/range", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("range {} 失败: {}", file_name, e))?;
        let bytes = resp.bytes().await.map_err(|e| format!("range 响应读取失败: {}", e))?;
        Ok(bytes.len() as u64)
    }

    /// 工作簿结构（POST /doc/workbook/structure）
    pub async fn workbook_structure(&self, file_id: &str, file_name: &str) -> Result<Value, String> {
        let body = serde_json::json!({ "file": { "fileId": file_id, "fileName": file_name } });
        let resp = self
            .client
            .post(format!("{}/doc/workbook/structure", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("structure {} 失败: {}", file_name, e))?;
        let json: Value = resp.json().await.map_err(|e| format!("structure 响应解析失败: {}", e))?;
        let raw = json["data"].as_str().unwrap_or("");
        if raw.is_empty() || raw == "{}" {
            return Err(format!("structure 响应为空: {}", file_name));
        }
        serde_json::from_str(raw).map_err(|e| format!("structure JSON 解析失败: {}", e))
    }

    /// 瓦片清单（POST /doc/workbook/sheet/tiles/manifest）
    pub async fn tile_manifest(&self, file_id: &str, file_name: &str, sheet_id: &str) -> Result<Value, String> {
        let body = serde_json::json!({
            "file": { "fileId": file_id, "fileName": file_name },
            "sheetId": sheet_id
        });
        let resp = self
            .client
            .post(format!("{}/doc/workbook/sheet/tiles/manifest", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("manifest {}#{} 失败: {}", file_name, sheet_id, e))?;
        let json: Value = resp.json().await.map_err(|e| format!("manifest 响应解析失败: {}", e))?;
        let raw = json["data"].as_str().unwrap_or("");
        serde_json::from_str(raw).map_err(|e| format!("manifest JSON 解析失败: {}", e))
    }

    /// 瓦片数据（POST /doc/workbook/sheet/tiles/data），返回瓦片数
    pub async fn tile_data(
        &self,
        file_id: &str,
        file_name: &str,
        sheet_id: &str,
        tile_ids: &[String],
    ) -> Result<usize, String> {
        let body = serde_json::json!({
            "file": { "fileId": file_id, "fileName": file_name },
            "sheetId": sheet_id,
            "tileIds": tile_ids
        });
        let resp = self
            .client
            .post(format!("{}/doc/workbook/sheet/tiles/data", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("tiles {}#{} 失败: {}", file_name, sheet_id, e))?;
        let json: Value = resp.json().await.map_err(|e| format!("tiles 响应解析失败: {}", e))?;
        let raw = json["data"].as_str().unwrap_or("");
        let map: Value = serde_json::from_str(raw).map_err(|e| format!("tiles JSON 解析失败: {}", e))?;
        let count = map.as_object().map(|m| m.len()).unwrap_or(0);
        if count == 0 {
            return Err(format!("tiles 响应为空: {}#{}", file_name, sheet_id));
        }
        Ok(count)
    }
}

/// 计时包装
pub async fn measure<F, T>(f: F) -> (T, u64)
where
    F: std::future::Future<Output = T>,
{
    let start = Instant::now();
    let result = f.await;
    (result, start.elapsed().as_millis() as u64)
}
