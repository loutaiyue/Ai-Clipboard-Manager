use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Emitter;
use tauri::Manager;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardEntryPayload {
    pub id: String,
    pub text: String,
    pub captured_at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct ClipboardHistoryFile {
    #[serde(default = "clipboard_history_version")]
    version: u32,
    items: Vec<ClipboardEntryPayload>,
}

fn clipboard_history_version() -> u32 {
    1
}

pub struct ClipboardStore {
    items: Arc<Mutex<Vec<ClipboardEntryPayload>>>,
    persist_tx: mpsc::Sender<()>,
}

impl ClipboardStore {
    pub fn new(items: Arc<Mutex<Vec<ClipboardEntryPayload>>>, persist_tx: mpsc::Sender<()>) -> Self {
        Self { items, persist_tx }
    }

    /// 仅更新内存并通知后台线程落盘（debounce），避免剪贴板线程被磁盘 IO 阻塞。
    pub fn push_notify(&self, entry: &ClipboardEntryPayload) {
        {
            let mut guard = self.items.lock().unwrap_or_else(|e| e.into_inner());
            guard.insert(0, entry.clone());
            if guard.len() > 200 {
                guard.truncate(200);
            }
        }
        let _ = self.persist_tx.send(());
    }
}

fn spawn_history_persist_worker(
    path: PathBuf,
    items: Arc<Mutex<Vec<ClipboardEntryPayload>>>,
    rx: mpsc::Receiver<()>,
) {
    thread::Builder::new()
        .name("clipboard-history-persist".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                // 合并短时间内的多次复制，降低磁盘写入与内存峰值克隆频率
                thread::sleep(Duration::from_millis(220));
                while rx.try_recv().is_ok() {}
                let snapshot = {
                    let guard = items.lock().unwrap_or_else(|e| e.into_inner());
                    guard.clone()
                };
                if let Err(e) = save_history_to_path(&path, snapshot) {
                    eprintln!("clipboard persist failed: {e}");
                }
            }
        })
        .expect("spawn clipboard-history-persist");
}

fn load_history_from_path(path: &Path) -> Vec<ClipboardEntryPayload> {
    let data = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    if let Ok(mut file) = serde_json::from_str::<ClipboardHistoryFile>(&data) {
        normalize_history_order(&mut file.items);
        return file.items;
    }
    if let Ok(mut items) = serde_json::from_str::<Vec<ClipboardEntryPayload>>(&data) {
        normalize_history_order(&mut items);
        return items;
    }
    vec![]
}

fn normalize_history_order(items: &mut Vec<ClipboardEntryPayload>) {
    items.sort_by(|a, b| b.captured_at_ms.cmp(&a.captured_at_ms));
    if items.len() > 200 {
        items.truncate(200);
    }
}

fn save_history_to_path(path: &Path, items: Vec<ClipboardEntryPayload>) -> Result<(), String> {
    let parent = path.parent().ok_or("无法解析历史文件目录")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    // 唯一临时文件名，避免多实例或崩溃残留导致写入冲突
    let tmp_name = format!(
        "{}.{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("clipboard_history.json"),
        std::process::id()
    );
    let tmp = parent.join(tmp_name);
    let wrapper = ClipboardHistoryFile { version: 1, items };
    let mut file = fs::File::create(&tmp).map_err(|e| format!("写入临时文件失败: {e}"))?;
    let write_result = (|| -> Result<(), String> {
        // 紧凑 JSON：体积更小、写入更快，降低高频复制时的窗口期
        serde_json::to_writer(&mut file, &wrapper).map_err(|e| format!("序列化历史失败: {e}"))?;
        file.flush().map_err(|e| format!("刷新临时文件失败: {e}"))?;
        file.sync_all().map_err(|e| format!("同步临时文件失败: {e}"))?;
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    drop(file);
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("替换历史文件失败: {e}"))?;
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("提交历史文件失败: {e}"));
    }
    Ok(())
}

fn millis_since_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 极短时间内相同文本只上报一次，避免个别应用连续触发多次剪贴板序号变化。
static CLIP_LAST_EMIT: Mutex<Option<(String, u64)>> = Mutex::new(None);

fn emit_text(app: &tauri::AppHandle, text: String, seq_hint: u32) {
    if text.trim().is_empty() {
        return;
    }
    let now = millis_since_epoch();
    {
        let mut g = CLIP_LAST_EMIT
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((ref prev, t)) = *g {
            if prev == &text && now.saturating_sub(t) < 240 {
                return;
            }
        }
        *g = Some((text.clone(), now));
    }
    let payload = ClipboardEntryPayload {
        id: format!("{now}-{seq_hint}"),
        text,
        captured_at_ms: now,
    };
    if let Some(store) = app.try_state::<ClipboardStore>() {
        store.push_notify(&payload);
    }
    let _ = app.emit("clipboard-text", payload);
}

fn spawn_clipboard_monitor(app: tauri::AppHandle) {
    thread::spawn(move || {
        #[cfg(windows)]
        {
            use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;

            let mut last_seq = unsafe { GetClipboardSequenceNumber() };
            loop {
                // 约 10 次/秒，便于从浏览器等应用复制后尽快出现在列表中
                thread::sleep(Duration::from_millis(100));
                let seq = unsafe { GetClipboardSequenceNumber() };
                if seq == last_seq {
                    continue;
                }
                last_seq = seq;
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    if let Ok(text) = clipboard.get_text() {
                        emit_text(&app, text, seq);
                    }
                }
            }
        }
        #[cfg(not(windows))]
        {
            let mut last_text = String::new();
            loop {
                thread::sleep(Duration::from_millis(200));
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    if let Ok(text) = clipboard.get_text() {
                        if text.trim().is_empty() || text == last_text {
                            continue;
                        }
                        last_text = text.clone();
                        emit_text(&app, text, 0);
                    }
                }
            }
        }
    });
}

fn normalize_chat_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.is_empty() {
        return "https://api.openai.com/v1/chat/completions".to_string();
    }
    if b.ends_with("/chat/completions") {
        b.to_string()
    } else {
        format!("{b}/chat/completions")
    }
}

fn normalize_anthropic_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.is_empty() {
        return "https://api.anthropic.com/v1/messages".to_string();
    }
    if b.ends_with("/messages") {
        b.to_string()
    } else {
        format!("{b}/messages")
    }
}

fn normalize_gemini_base(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.is_empty() {
        "https://generativelanguage.googleapis.com/v1beta".to_string()
    } else {
        b.to_string()
    }
}

fn parse_error_message(raw: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return Some(msg.to_string());
        }
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("type"))
            .and_then(|m| m.as_str())
        {
            return Some(msg.to_string());
        }
    }
    None
}

async fn summarize_openai_compatible(
    client: &reqwest::Client,
    text: &str,
    instruction: &str,
    api_base: &str,
    api_key: &str,
    model: &str,
) -> Result<String, String> {
    let url = normalize_chat_url(api_base);
    let body = json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": instruction
            },
            { "role": "user", "content": text }
        ],
        "temperature": 0.35
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败（请检查网络与 API 地址）: {e}"))?;

    let status = resp.status();
    let raw = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
    if !status.is_success() {
        if let Some(msg) = parse_error_message(&raw) {
            return Err(format!("API 错误 ({status}): {msg}"));
        }
        return Err(format!("API 返回 {status}: {}", raw.chars().take(280).collect::<String>()));
    }

    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("解析 JSON 失败: {e}"))?;
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "响应中未找到 choices[0].message.content，请确认接口为 OpenAI Chat Completions 格式。".to_string())?;
    Ok(content.trim().to_string())
}

async fn summarize_anthropic(
    client: &reqwest::Client,
    text: &str,
    instruction: &str,
    api_base: &str,
    api_key: &str,
    model: &str,
) -> Result<String, String> {
    let url = normalize_anthropic_url(api_base);
    let body = json!({
        "model": model,
        "max_tokens": 800,
        "temperature": 0.35,
        "system": instruction,
        "messages": [
            { "role": "user", "content": text }
        ]
    });

    let resp = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败（请检查网络与 API 地址）: {e}"))?;

    let status = resp.status();
    let raw = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
    if !status.is_success() {
        if let Some(msg) = parse_error_message(&raw) {
            return Err(format!("Anthropic 错误 ({status}): {msg}"));
        }
        return Err(format!("Anthropic 返回 {status}: {}", raw.chars().take(280).collect::<String>()));
    }

    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("解析 JSON 失败: {e}"))?;
    let content = v
        .pointer("/content/0/text")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "响应中未找到 content[0].text，请确认接口为 Anthropic Messages 格式。".to_string())?;
    Ok(content.trim().to_string())
}

async fn summarize_gemini(
    client: &reqwest::Client,
    text: &str,
    instruction: &str,
    api_base: &str,
    api_key: &str,
    model: &str,
) -> Result<String, String> {
    let base = normalize_gemini_base(api_base);
    let model_id = if model.trim().is_empty() { "gemini-2.0-flash" } else { model.trim() };
    let url = format!("{base}/models/{model_id}:generateContent?key={api_key}");
    let body = json!({
        "system_instruction": {
            "parts": [
                { "text": instruction }
            ]
        },
        "contents": [
            {
                "role": "user",
                "parts": [{ "text": text }]
            }
        ],
        "generationConfig": {
            "temperature": 0.35
        }
    });

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败（请检查网络与 API 地址）: {e}"))?;

    let status = resp.status();
    let raw = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
    if !status.is_success() {
        if let Some(msg) = parse_error_message(&raw) {
            return Err(format!("Gemini 错误 ({status}): {msg}"));
        }
        return Err(format!("Gemini 返回 {status}: {}", raw.chars().take(280).collect::<String>()));
    }

    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("解析 JSON 失败: {e}"))?;
    let content = v
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "响应中未找到 candidates[0].content.parts[0].text，请确认接口为 Gemini generateContent 格式。".to_string())?;
    Ok(content.trim().to_string())
}

fn instruction_for_task(task: &str) -> &'static str {
    match task.trim() {
        "translate_polish" => {
            "你是中英双语编辑。请将用户文本翻译成自然、专业、流畅的中文（若原文已是中文则润色重写为更清晰版本）。输出结构：1) 润色后文本；2) 关键优化点（3-5条）。不要输出与任务无关内容。"
        }
        "code_diagnose" => {
            "你是资深代码审查工程师。请识别这段内容中的代码问题与风险：语法/逻辑错误、潜在异常、安全风险、性能问题、可维护性问题。输出结构：1) 问题列表；2) 修复建议；3) 如适用，给出简短修复示例。"
        }
        "extract_key_elements" => {
            "你是信息提取助手。请从文本中提取关键要素，按条目输出：主题、人物/实体、时间、地点、关键数据、行动项、待确认点。没有的信息写“未提及”。"
        }
        _ => {
            "你是一个中文助手。请用简洁的中文总结用户提供的剪贴板文本：抓住要点，条理清晰，不要开场白和客套结尾。"
        }
    }
}

#[tauri::command]
fn get_clipboard_history(state: tauri::State<ClipboardStore>) -> Vec<ClipboardEntryPayload> {
    state
        .items
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|e| e.into_inner().clone())
}

/// 多提供商总结接口（OpenAI 兼容、Anthropic、Gemini），由 Rust 发起请求以避免浏览器 CORS。
#[tauri::command]
async fn ai_summarize(
    text: String,
    task: String,
    provider: String,
    api_base: String,
    api_key: String,
    model: String,
) -> Result<String, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("请先在右侧填写 API Key。".into());
    }
    let text = text.trim();
    if text.is_empty() {
        return Err("当前没有可总结的文本。".into());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))?;
    let instruction = instruction_for_task(&task);
    let p = provider.trim();
    match p {
        "anthropic" => {
            let m = if model.trim().is_empty() {
                "claude-3-5-sonnet-latest"
            } else {
                model.trim()
            };
            summarize_anthropic(&client, &text, instruction, &api_base, api_key, m).await
        }
        "gemini" => summarize_gemini(&client, &text, instruction, &api_base, api_key, &model).await,
        _ => {
            let m = if model.trim().is_empty() {
                "gpt-4o-mini"
            } else {
                model.trim()
            };
            summarize_openai_compatible(&client, &text, instruction, &api_base, api_key, m).await
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("无法定位应用数据目录: {e}"))?;
            fs::create_dir_all(&data_dir).map_err(|e| format!("创建应用数据目录失败: {e}"))?;
            let history_path = data_dir.join("clipboard_history.json");
            let loaded = load_history_from_path(&history_path);
            let items = Arc::new(Mutex::new(loaded));
            let (persist_tx, persist_rx) = mpsc::channel::<()>();
            spawn_history_persist_worker(history_path.clone(), items.clone(), persist_rx);
            app.manage(ClipboardStore::new(items, persist_tx));
            spawn_clipboard_monitor(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![ai_summarize, get_clipboard_history])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
