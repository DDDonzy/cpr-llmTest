use gateway_plugin_sdk::{
    ErrorCode, Manifest, PluginFault,
    call::{
        data::{AccountFacts, AccountFactsQuery},
        host::{
            KeyListRequest, ModelEventBatch, ModelExecuteRequest, ModelListRequest, ModelOperation,
            ModelStreamCloseRequest, ModelStreamReadRequest, ModelStreamReadResult,
            ModelStreamResult, StateGetRequest, StateGetResult, StatePutRequest, StatePutResult,
        },
        management::{
            ManagementPage, ManagementRegistration, ManagementRequest, ManagementResource,
            ManagementResponse, ManagementRoute,
        },
        middleware::{MiddlewareHeader, http::Version},
        model::{CanonicalEvent, FinishReason, WirePayload},
    },
    client::{
        AuthorError, ComposedPlugin, HostClient, HttpBody, HttpFrame, HttpRequest, PluginBuilder,
        SessionError, TypedCall, TypedReply,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
const MAX_TASKS: usize = 500;
const MAX_TEXT: usize = 64 * 1024;
const LEVELS: &[&str] = &[
    "default", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];
pub fn manifest() -> Result<Manifest, gateway_plugin_sdk::ManifestError> {
    Manifest::from_author_slice(include_bytes!("../../plugin.json"))
}
fn fault(code: ErrorCode, msg: &str) -> PluginFault {
    PluginFault::new(code, msg)
}
fn reply(status: u16, v: Value) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    Ok(TypedReply::new(ManagementResponse {
        status,
        content_type: "application/json".into(),
        headers: vec![MiddlewareHeader {
            name: "cache-control".into(),
            value: "no-store".into(),
        }],
    })
    .with_payload(serde_json::to_vec(&v).map_err(|_| fault(ErrorCode::Fault, "响应编码失败"))?))
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, PluginFault> {
    serde_json::from_slice(bytes)
        .map_err(|_| fault(ErrorCode::InvalidInput, "请求字段或 JSON 无效"))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub key_id: String,
    pub accounts: Vec<String>,
    pub models: Vec<String>,
    pub efforts: Vec<String>,
    pub prompt: String,
    pub concurrency: usize,
    pub timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_task_indexes: Option<Vec<usize>>,
}
impl Plan {
    fn validate(&self) -> Result<(), PluginFault> {
        if self.key_id.is_empty()
            || self.key_id.len() > 200
            || self.prompt.trim().is_empty()
            || self.prompt.len() > 16384
            || self.concurrency == 0
            || self.timeout_seconds < 10
        {
            return Err(fault(
                ErrorCode::InvalidInput,
                "问题不能为空；并发至少 1，超时至少 10 秒",
            ));
        }
        for xs in [&self.accounts, &self.models, &self.efforts] {
            if xs.is_empty()
                || xs.len() > MAX_TASKS
                || xs.iter().collect::<HashSet<_>>().len() != xs.len()
            {
                return Err(fault(
                    ErrorCode::InvalidInput,
                    "请选择账号、模型和思考等级，不允许重复",
                ));
            }
        }
        if self.models.iter().any(|s| {
            s.trim() != s || s.is_empty() || s.len() > 200 || s.chars().any(char::is_control)
        }) || self.efforts.iter().any(|s| !LEVELS.contains(&s.as_str()))
        {
            return Err(fault(ErrorCode::InvalidInput, "模型名或思考等级无效"));
        }
        let count = self
            .accounts
            .len()
            .checked_mul(self.models.len())
            .and_then(|x| x.checked_mul(self.efforts.len()));
        if count.is_none_or(|x| x > MAX_TASKS) {
            return Err(fault(ErrorCode::Capacity, "每批最多 500 条任务"));
        }
        if let Some(indexes) = &self.selected_task_indexes
            && (indexes.is_empty()
                || indexes.len() > MAX_TASKS
                || indexes.iter().collect::<HashSet<_>>().len() != indexes.len()
                || indexes.iter().any(|&i| i >= count.unwrap()))
        {
            return Err(fault(
                ErrorCode::InvalidInput,
                "请至少选择一项测试任务，任务索引不能重复或超出组合范围",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preset {
    id: String,
    name: String,
    prompt: String,
}
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetDocument {
    presets: Vec<Preset>,
}
impl PresetDocument {
    fn validate(&self) -> Result<(), PluginFault> {
        let ids: HashSet<_> = self.presets.iter().map(|p| &p.id).collect();
        let names: HashSet<_> = self.presets.iter().map(|p| p.name.trim()).collect();
        if self.presets.len() > 32
            || ids.len() != self.presets.len()
            || names.len() != self.presets.len()
            || self.presets.iter().any(|p| {
                p.id.is_empty()
                    || p.id.len() > 80
                    || !p
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                    || p.name.trim().is_empty()
                    || p.name.chars().count() > 64
                    || p.name.chars().any(char::is_control)
                    || p.prompt.trim().is_empty()
                    || p.prompt.len() > 16384
            })
            || serde_json::to_vec(self).map_or(true, |b| b.len() > 48 * 1024)
        {
            return Err(fault(
                ErrorCode::InvalidInput,
                "预设名称或问题无效、重复，或超出32条/48KiB限制",
            ));
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavePresets {
    expected_version: Option<u64>,
    presets: Vec<Preset>,
}
async fn get_presets(host: &HostClient) -> Result<Value, PluginFault> {
    let r = host
        .call(
            "host.state.get",
            json!(StateGetRequest {
                namespace: "prompt-presets".into(),
                key: "document".into()
            }),
            vec![],
        )
        .await
        .map_err(SessionError::into_plugin_fault)?;
    let r: StateGetResult =
        serde_json::from_value(r.result).map_err(|_| fault(ErrorCode::Fault, "预设状态无效"))?;
    match r.record {
        None => Ok(json!({"version":null,"presets":[]})),
        Some(r) => {
            if r.schema_version != 1 {
                return Err(fault(ErrorCode::Unsupported, "预设状态版本不兼容"));
            }
            let doc: PresetDocument = serde_json::from_value(r.value)
                .map_err(|_| fault(ErrorCode::Fault, "预设数据无效"))?;
            doc.validate()?;
            Ok(json!({"version":r.version,"presets":doc.presets}))
        }
    }
}
async fn save_presets(host: &HostClient, req: SavePresets) -> Result<Value, PluginFault> {
    let doc = PresetDocument {
        presets: req.presets,
    };
    doc.validate()?;
    let r = host
        .call(
            "host.state.put",
            json!(StatePutRequest {
                namespace: "prompt-presets".into(),
                key: "document".into(),
                expected_version: req.expected_version,
                value: json!(doc)
            }),
            vec![],
        )
        .await
        .map_err(SessionError::into_plugin_fault)?;
    let saved: StatePutResult = serde_json::from_value(r.result)
        .map_err(|_| fault(ErrorCode::Fault, "预设保存响应无效"))?;
    Ok(json!({"version":saved.version,"presets":doc.presets}))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    id: usize,
    account_id: String,
    account_name: String,
    provider: String,
    model: String,
    effort: String,
    status: String,
    result: Value,
}
#[derive(Clone)]
struct Job {
    id: u64,
    plan: Plan,
    tasks: Vec<Task>,
    paused: bool,
}
impl Job {
    fn summary(&self) -> Value {
        json!({"id":self.id.to_string(),"plan":self.plan,"paused":self.paused,"tasks":self.tasks.iter().map(|t|json!({"id":t.id,"accountId":t.account_id,"accountName":t.account_name,"model":t.model,"effort":t.effort,"status":t.status,"durationMs":t.result["durationMs"],"verification":t.result["verification"],"error":t.result["error"]})).collect::<Vec<_>>()})
    }
    fn active(&self) -> usize {
        self.tasks.iter().filter(|t| t.status == "running").count()
    }
}
#[derive(Default)]
struct State {
    next: u64,
    job: Option<Job>,
}
#[derive(Default, Clone)]
struct App {
    state: Arc<Mutex<State>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ref {
    job_id: String,
    task_id: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchRef {
    job_id: String,
    task_ids: Vec<usize>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Control {
    job_id: String,
    action: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Key {
    key_id: String,
}
fn job_mut<'a>(s: &'a mut State, id: &str) -> Result<&'a mut Job, PluginFault> {
    s.job
        .as_mut()
        .filter(|j| j.id.to_string() == id)
        .ok_or_else(|| fault(ErrorCode::Conflict, "任务批次已失效，请重新加载"))
}
// 在父调用取消、超时或页面断连时，不允许同一任务自动重新发送。
struct RunningGuard {
    state: Arc<Mutex<State>>,
    job: String,
    task: usize,
    done: bool,
}
impl Drop for RunningGuard {
    fn drop(&mut self) {
        if !self.done {
            let mut s = self.state.lock().unwrap();
            if let Ok(j) = job_mut(&mut s, &self.job)
                && let Some(t) = j.tasks.get_mut(self.task)
                && t.status == "running"
            {
                t.status = "unknown".into();
                t.result["error"] = json!({"code":"uncertain","message":"父调用中断，执行结果未知。不要自动重试，请核对 CPR 请求日志。"})
            }
        }
    }
}
async fn accounts(host: &HostClient) -> Result<Vec<AccountFacts>, PluginFault> {
    let mut out = vec![];
    let mut cursor = None;
    for _ in 0..20 {
        let r = host
            .account_facts(AccountFactsQuery {
                provider_id: None,
                cursor,
                limit: 100,
            })
            .await?;
        out.extend(r.accounts);
        cursor = r.next_cursor;
        if cursor.is_none() {
            return Ok(out);
        }
    }
    Err(fault(ErrorCode::Capacity, "账号数量超过分页上限"))
}
async fn keys(host: &HostClient) -> Result<Value, PluginFault> {
    let mut out = vec![];
    let mut cursor = None;
    for _ in 0..20 {
        let r = host
            .list_keys(KeyListRequest { cursor, limit: 100 })
            .await?;
        out.extend(r.keys);
        cursor = r.next_cursor;
        if cursor.is_none() {
            return Ok(json!(out));
        }
    }
    Err(fault(ErrorCode::Capacity, "Key 数量超过分页上限"))
}
async fn native(host: &HostClient, uri: String) -> Result<Value, PluginFault> {
    let mut r = host
        .dispatch_http(HttpRequest {
            method: "GET".into(),
            uri,
            version: Version::Http11,
            headers: vec![],
            settings: Value::Null,
            timeout_ms: Some(5000),
            body: HttpBody::empty(),
        })
        .await?;
    let mut b = vec![];
    while let Some(f) = r.body.read().await? {
        if let HttpFrame::Data(x) = f {
            if b.len() + x.len() > 1024 * 1024 {
                r.body.close().await?;
                return Err(fault(ErrorCode::Capacity, "日志响应过大"));
            }
            b.extend(x)
        }
    }
    if r.status != 200 {
        return Err(fault(ErrorCode::Fault, "日志暂未可用"));
    }
    let v: Value = decode(&b)?;
    if v["code"] != 200 {
        return Err(fault(ErrorCode::Fault, "日志暂未可用"));
    }
    Ok(v["data"].clone())
}
fn push_text(dst: &mut String, text: &str) -> bool {
    let space = MAX_TEXT.saturating_sub(dst.len());
    let mut n = text.len().min(space);
    while !text.is_char_boundary(n) {
        n -= 1
    }
    dst.push_str(&text[..n]);
    n < text.len()
}
fn consume_wire(v: &Value, result: &mut Value, text: &mut String, usage_seen: &mut bool) {
    let typ = v["type"].as_str().unwrap_or("");
    if typ == "response.output_text.delta"
        && let Some(s) = v["delta"].as_str()
        && push_text(text, s)
    {
        result["truncated"] = json!(true)
    }
    let response = if typ == "response.completed"
        || typ == "response.incomplete"
        || typ == "response.failed"
    {
        &v["response"]
    } else {
        v
    };
    if response["object"] == "response" || response.get("output").is_some() {
        if let Some(model) = response["model"].as_str() {
            result["responseModel"] = json!(model)
        }
        if let Some(status) = response["status"].as_str() {
            result["responseStatus"] = json!(status)
        }
        if text.is_empty()
            && let Some(output) = response["output"].as_array()
        {
            for item in output {
                if let Some(content) = item["content"].as_array() {
                    for part in content {
                        if part["type"] == "output_text"
                            && let Some(s) = part["text"].as_str()
                            && push_text(text, s)
                        {
                            result["truncated"] = json!(true)
                        }
                    }
                }
            }
        }
        if !*usage_seen && response["usage"].is_object() {
            result["usage"] = response["usage"].clone()
        }
        // 只表示上游回显，不把它标为实际思考强度证明。
        if let Some(effort) = response["reasoning"]["effort"].as_str() {
            result["effortEcho"] = json!(effort)
        }
    }
    if typ == "error" || typ == "response.failed" {
        result["upstreamError"] = json!(true)
    }
}
async fn execute(
    host: &HostClient,
    plan: &Plan,
    t: &Task,
    progress: impl Fn(Value),
) -> Result<Value, PluginFault> {
    let start = Instant::now();
    let mut body = json!({"model":t.model,"input":[{"role":"user","content":[{"type":"input_text","text":plan.prompt}]}],"stream":true,"store":false});
    if t.effort != "default" {
        body["reasoning"] = json!({"effort":t.effort})
    }
    let req = ModelExecuteRequest {
        client_key_id: Some(plan.key_id.clone()),
        model: t.model.clone(),
        protocol: "openai".into(),
        operation: ModelOperation::Generate,
        provider: Some(t.provider.clone()),
        account_id: Some(t.account_id.clone()),
        previous_response_id: None,
    };
    let r = host
        .call(
            "host.model.execute_stream",
            serde_json::to_value(req).unwrap(),
            serde_json::to_vec(&body).unwrap(),
        )
        .await
        .map_err(SessionError::into_plugin_fault)?;
    let stream: ModelStreamResult =
        serde_json::from_value(r.result).map_err(|_| fault(ErrorCode::Fault, "模型流响应无效"))?;
    let mut result = json!({"requestId":stream.request_id,"requestedAccountId":t.account_id,"verification":"unverified","requestedEffort":t.effort,"effortEcho":null,"text":"","usage":null,"durationMs":0,"firstTextMs":null,"truncated":false,"complete":false});
    progress(result.clone());
    let mut text = String::new();
    let mut completed = false;
    let mut usage_seen = false;
    let loop_result = async {
        loop {
            let r = host
                .call(
                    "host.model.stream_read",
                    serde_json::to_value(ModelStreamReadRequest {
                        stream: stream.stream.clone(),
                        maximum_bytes: 1024 * 1024,
                    })
                    .unwrap(),
                    vec![],
                )
                .await
                .map_err(SessionError::into_plugin_fault)?;
            let meta: ModelStreamReadResult = serde_json::from_value(r.result)
                .map_err(|_| fault(ErrorCode::Fault, "模型事件元数据无效"))?;
            if !r.payload.is_empty() {
                let batch = ModelEventBatch::decode(&r.payload)
                    .map_err(|_| fault(ErrorCode::Fault, "模型事件载荷无效"))?;
                for e in batch.events {
                    let canonical_text = e
                        .facts
                        .iter()
                        .any(|f| matches!(f, CanonicalEvent::TextDelta { .. }));
                    for f in e.facts {
                        match f {
                            CanonicalEvent::TextDelta { text: s, .. } => {
                                if push_text(&mut text, &s) {
                                    result["truncated"] = json!(true)
                                }
                            }
                            CanonicalEvent::Usage { usage } => {
                                result["usage"] = json!(usage);
                                usage_seen = true
                            }
                            CanonicalEvent::Started { model: Some(m), .. } => {
                                result["responseModel"] = json!(m);
                            }
                            CanonicalEvent::Completed { model, reason, .. } => {
                                completed = true;
                                result["finishReason"] = json!(reason);
                                result["complete"] = json!(reason == FinishReason::Stop);
                                if let Some(m) = model {
                                    result["responseModel"] = json!(m)
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(h) = e.host {
                        if let Some(obs) = h.observation {
                            result["upstreamStatus"] = json!(obs.status_code);
                            if obs.upstream_response_model.is_some() {
                                result["responseModel"] = json!(obs.upstream_response_model)
                            }
                        }
                        if !h.costs.is_empty() {
                            result["costs"] = json!(h.costs)
                        }
                    }
                    if let Some(w) = e.wire
                        && !canonical_text
                    {
                        match w.payload {
                            WirePayload::Json { data, .. } => {
                                consume_wire(&data, &mut result, &mut text, &mut usage_seen)
                            }
                            WirePayload::RawJson { body } => {
                                if let Ok(v) = serde_json::from_slice::<Value>(&body) {
                                    consume_wire(&v, &mut result, &mut text, &mut usage_seen)
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if !text.is_empty() && result["firstTextMs"].is_null() {
                result["firstTextMs"] = json!(start.elapsed().as_millis() as u64)
            }
            result["text"] = json!(text);
            result["durationMs"] = json!(start.elapsed().as_millis() as u64);
            progress(result.clone());
            if meta.end {
                break;
            }
        }
        Ok::<(), PluginFault>(())
    }
    .await;
    if loop_result.is_err() {
        let _ = host
            .call(
                "host.model.stream_close",
                serde_json::to_value(ModelStreamCloseRequest {
                    stream: stream.stream,
                })
                .unwrap(),
                vec![],
            )
            .await;
    }
    loop_result?;
    if !completed {
        result["complete"] = json!(result["responseStatus"] == "completed")
    }
    result["durationMs"] = json!(start.elapsed().as_millis() as u64);
    Ok(result)
}
async fn audit(host: &HostClient, result: &mut Value, expected: &str) -> Result<(), PluginFault> {
    let id = result["requestId"]
        .as_str()
        .ok_or_else(|| fault(ErrorCode::InvalidInput, "任务没有可核对的请求 ID"))?;
    if id.is_empty()
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(fault(ErrorCode::Fault, "请求 ID 无效"));
    }
    let data = native(host, format!("/api/admin/usage/records/detail?id={id}")).await?;
    if data["requestId"] != id {
        return Err(fault(ErrorCode::Fault, "日志请求 ID 不匹配"));
    }
    let mut ids = HashSet::new();
    if let Some(a) = data["accountId"].as_str() {
        ids.insert(a.to_owned());
    }
    if let Some(attempts) = data["attempts"].as_array() {
        for a in attempts {
            if let Some(id) = a["accountId"].as_str() {
                ids.insert(id.to_owned());
            }
        }
    }
    result["actualAccountIds"] = json!(ids.iter().collect::<Vec<_>>());
    result["verification"] = json!(if ids.iter().any(|id| id != expected) {
        "mismatch"
    } else if ids.len() == 1 && data["attemptsComplete"] == true {
        "verified"
    } else if ids.len() == 1 {
        "matched"
    } else {
        "unverified"
    });
    result["attemptsComplete"] = data["attemptsComplete"].clone();
    result["upstreamStatus"] = data["upstreamStatusCode"].clone();
    result["loggedEffort"] = data["reasoningEffort"].clone();
    result["logicalOutcome"] = data["logicalOutcome"].clone();
    Ok(())
}
impl App {
    async fn handle(
        &self,
        call: TypedCall<ManagementRequest>,
    ) -> Result<TypedReply<ManagementResponse>, PluginFault> {
        if !call.request.query.is_empty() || call.payload.len() > 64 * 1024 {
            return Err(fault(ErrorCode::InvalidInput, "查询参数或正文过大"));
        }
        match (call.request.method.as_str(), call.request.path.as_str()) {
            ("GET", "api/snapshot") => {
                let a = accounts(&call.host).await?;
                let k = keys(&call.host).await?;
                let s = self.state.lock().unwrap();
                reply(
                    200,
                    json!({"accounts":a,"keys":k,"efforts":LEVELS,"job":s.job.as_ref().map(Job::summary),"version":env!("CARGO_PKG_VERSION"),"managementTimeoutSeconds":call.context.timeout_ms/1000}),
                )
            }
            ("GET", "api/presets") => reply(200, get_presets(&call.host).await?),
            ("POST", "api/presets") => {
                reply(200, save_presets(&call.host, decode(&call.payload)?).await?)
            }
            ("POST", "api/models") => {
                let k: Key = decode(&call.payload)?;
                let r = call
                    .host
                    .call(
                        "host.models.list",
                        serde_json::to_value(ModelListRequest {
                            client_key_id: k.key_id,
                            protocol: "openai".into(),
                            client_version: String::new(),
                        })
                        .unwrap(),
                        vec![],
                    )
                    .await
                    .map_err(SessionError::into_plugin_fault)?;
                reply(200, r.result)
            }
            ("POST", "api/plan") => {
                let plan: Plan = decode(&call.payload)?;
                plan.validate()?;
                let a = accounts(&call.host).await?;
                let k = keys(&call.host).await?;
                if !k
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|k| k["id"] == plan.key_id && k["enabled"] == true)
                {
                    return Err(fault(ErrorCode::InvalidInput, "请选择启用的执行 Key"));
                }
                let mut tasks = vec![];
                let selected = plan
                    .selected_task_indexes
                    .as_ref()
                    .map(|xs| xs.iter().copied().collect::<HashSet<_>>());
                let mut matrix_index = 0;
                for id in &plan.accounts {
                    let a = a
                        .iter()
                        .find(|a| a.account_id == *id && a.enabled)
                        .ok_or_else(|| {
                            fault(ErrorCode::InvalidInput, "选中的账号已不存在或已停用")
                        })?;
                    if a.provider_id != "openai" && a.provider_id != "xai" {
                        return Err(fault(
                            ErrorCode::Unsupported,
                            "此版本只测试 Responses 兼容账号",
                        ));
                    }
                    for model in &plan.models {
                        for effort in &plan.efforts {
                            let included = selected
                                .as_ref()
                                .is_none_or(|xs| xs.contains(&matrix_index));
                            matrix_index += 1;
                            if !included {
                                continue;
                            }
                            tasks.push(Task {
                                id: tasks.len(),
                                account_id: id.clone(),
                                account_name: a.email.clone().unwrap_or_else(|| a.name.clone()),
                                provider: a.provider_id.clone(),
                                model: model.clone(),
                                effort: effort.clone(),
                                status: "pending".into(),
                                result: json!({}),
                            })
                        }
                    }
                }
                let mut s = self.state.lock().unwrap();
                if s.job.as_ref().is_some_and(|j| {
                    j.active() > 0 || j.tasks.iter().any(|t| t.status == "pending")
                }) {
                    return reply(
                        409,
                        json!({"error":{"message":"上一批仍有运行中或待测任务，请先取消待测项并等待执行结束"}}),
                    );
                }
                s.next += 1;
                s.job = Some(Job {
                    id: s.next,
                    plan,
                    tasks,
                    paused: true,
                });
                reply(200, s.job.as_ref().unwrap().summary())
            }
            ("GET", "api/job") => {
                let s = self.state.lock().unwrap();
                reply(200, json!({"job":s.job.as_ref().map(Job::summary)}))
            }
            ("POST", "api/control") => {
                let q: Control = decode(&call.payload)?;
                let mut s = self.state.lock().unwrap();
                let j = job_mut(&mut s, &q.job_id)?;
                match q.action.as_str() {
                    "start" => j.paused = false,
                    "pause" => j.paused = true,
                    "cancel" => {
                        j.paused = true;
                        for t in &mut j.tasks {
                            if t.status == "pending" {
                                t.status = "cancelled".into()
                            }
                        }
                    }
                    _ => return Err(fault(ErrorCode::InvalidInput, "控制动作无效")),
                }
                reply(200, j.summary())
            }
            ("POST", "api/task") => {
                let q: Ref = decode(&call.payload)?;
                let mut s = self.state.lock().unwrap();
                let j = job_mut(&mut s, &q.job_id)?;
                let t = j
                    .tasks
                    .get(q.task_id)
                    .ok_or_else(|| fault(ErrorCode::InvalidInput, "任务不存在"))?;
                reply(200, json!(t))
            }
            ("POST", "api/audit") => {
                let q: Ref = decode(&call.payload)?;
                let (mut result, account) = {
                    let mut s = self.state.lock().unwrap();
                    let j = job_mut(&mut s, &q.job_id)?;
                    let t = j
                        .tasks
                        .get(q.task_id)
                        .ok_or_else(|| fault(ErrorCode::InvalidInput, "任务不存在"))?;
                    if t.status == "running" || t.status == "pending" {
                        return Err(fault(ErrorCode::Conflict, "请等待执行结束再核对"));
                    }
                    (t.result.clone(), t.account_id.clone())
                };
                audit(&call.host, &mut result, &account).await?;
                let mut s = self.state.lock().unwrap();
                let j = job_mut(&mut s, &q.job_id)?;
                let t = &mut j.tasks[q.task_id];
                t.result = result;
                reply(200, json!(t))
            }
            ("POST", "api/run") => {
                self.run(&call.host, call.context.timeout_ms, decode(&call.payload)?)
                    .await
            }
            ("POST", "api/run-batch") => self.run_batch(call).await,
            _ => reply(404, json!({"error":{"message":"接口不存在"}})),
        }
    }
    async fn run_batch(
        &self,
        call: TypedCall<ManagementRequest>,
    ) -> Result<TypedReply<ManagementResponse>, PluginFault> {
        let q: BatchRef = decode(&call.payload)?;
        if q.task_ids.is_empty()
            || q.task_ids.len() > MAX_TASKS
            || q.task_ids.iter().collect::<HashSet<_>>().len() != q.task_ids.len()
        {
            return Err(fault(ErrorCode::InvalidInput, "批次任务不能为空或重复"));
        }
        {
            let mut s = self.state.lock().unwrap();
            let j = job_mut(&mut s, &q.job_id)?;
            if q.task_ids.iter().any(|&id| id >= j.tasks.len()) {
                return Err(fault(ErrorCode::InvalidInput, "批次包含不存在的任务"));
            }
            let pending = q
                .task_ids
                .iter()
                .filter(|&&id| j.tasks[id].status == "pending")
                .count();
            if j.paused {
                return reply(409, json!({"error":{"message":"任务已暂停"}}));
            }
            if pending > j.plan.concurrency.saturating_sub(j.active()) {
                return reply(
                    429,
                    json!({"error":{"message":"批次超过所选并发，未发送请求"}}),
                );
            }
        }
        // One parent management call owns every child task. JoinSet drop aborts
        // children on parent cancellation; RunningGuard records unknown, never replay.
        let mut runs = tokio::task::JoinSet::new();
        for id in q.task_ids {
            let app = self.clone();
            let host = call.host.clone();
            let timeout_ms = call.context.timeout_ms;
            let job_id = q.job_id.clone();
            runs.spawn(async move {
                (
                    id,
                    app.run(
                        &host,
                        timeout_ms,
                        Ref {
                            job_id,
                            task_id: id,
                        },
                    )
                    .await,
                )
            });
        }
        let mut issues = vec![];
        while let Some(result) = runs.join_next().await {
            match result {
                Ok((id, Ok(reply))) if reply.result.status >= 400 => {
                    let v: Value = serde_json::from_slice(&reply.payload).unwrap_or_default();
                    issues.push(json!({"taskId":id,"message":v["error"]["message"].as_str().unwrap_or("任务未执行")}));
                }
                Ok((id, Err(e))) => issues.push(json!({"taskId":id,"message":safe_error(&e)})),
                Err(_) => issues.push(json!({"taskId":null,"message":"任务中断，不会自动重试"})),
                _ => {}
            }
        }
        let mut s = self.state.lock().unwrap();
        let j = job_mut(&mut s, &q.job_id)?;
        // Summary only: 500 full answers can exceed the host's 1 MiB reply budget.
        reply(200, json!({"job":j.summary(),"issues":issues}))
    }
    async fn run(
        &self,
        host: &HostClient,
        context_timeout_ms: u64,
        q: Ref,
    ) -> Result<TypedReply<ManagementResponse>, PluginFault> {
        let (plan, task) = {
            let mut s = self.state.lock().unwrap();
            let j = job_mut(&mut s, &q.job_id)?;
            let t = j
                .tasks
                .get(q.task_id)
                .ok_or_else(|| fault(ErrorCode::InvalidInput, "任务不存在"))?;
            if t.status != "pending" {
                return reply(if t.status == "running" { 409 } else { 200 }, json!(t));
            }
            if j.paused {
                return reply(409, json!({"error":{"message":"任务已暂停"}}));
            }
            if j.active() >= j.plan.concurrency {
                return reply(
                    429,
                    json!({"error":{"message":"已达到批次并发上限，未发送请求"}}),
                );
            }
            if context_timeout_ms < 15_000 {
                return Err(fault(
                    ErrorCode::Timeout,
                    "宿主管理调用期限过短，未发送请求",
                ));
            }
            let plan = j.plan.clone();
            let t = &mut j.tasks[q.task_id];
            t.status = "running".into();
            (plan, t.clone())
        };
        let mut guard = RunningGuard {
            state: self.state.clone(),
            job: q.job_id.clone(),
            task: q.task_id,
            done: false,
        };
        let state = self.state.clone();
        let id = q.job_id.clone();
        let taskid = q.task_id;
        let progress = move |result: Value| {
            let mut s = state.lock().unwrap();
            if let Ok(j) = job_mut(&mut s, &id) {
                j.tasks[taskid].result = result;
            }
        };
        let timeout = plan
            .timeout_seconds
            .min(context_timeout_ms.saturating_sub(8000) / 1000);
        let start = Instant::now();
        let outcome = tokio::time::timeout(
            Duration::from_secs(timeout),
            execute(host, &plan, &task, progress),
        )
        .await;
        let (mut result, status) = match outcome {
            Ok(Ok(r)) => {
                let status = if r["upstreamError"] == true {
                    "error"
                } else if r["complete"] == true && !r["text"].as_str().unwrap_or("").is_empty() {
                    "success"
                } else {
                    "incomplete"
                };
                (r, status)
            }
            e => {
                let mut s = self.state.lock().unwrap();
                let j = job_mut(&mut s, &q.job_id)?;
                let mut r = j.tasks[q.task_id].result.clone();
                if !r.is_object() {
                    r = json!({})
                };
                let (code, msg, status) = match e {
                    Ok(Err(f)) => (
                        format!("{:?}", f.code).to_lowercase(),
                        safe_error(&f),
                        "error",
                    ),
                    Err(_) => (
                        "timeout".into(),
                        "达到测试超时，可能已消耗额度；不自动重试".into(),
                        "timeout",
                    ),
                    _ => unreachable!(),
                };
                r["error"] = json!({"code":code,"message":msg});
                r["durationMs"] = json!(start.elapsed().as_millis() as u64);
                (r, status)
            }
        };
        result["configuredTimeoutSeconds"] = json!(plan.timeout_seconds);
        result["effectiveTimeoutSeconds"] = json!(timeout);
        if result["requestId"].is_string() {
            let _ = tokio::time::timeout(
                Duration::from_secs(4),
                audit(host, &mut result, &task.account_id),
            )
            .await;
        }
        let mut s = self.state.lock().unwrap();
        let j = job_mut(&mut s, &q.job_id)?;
        let t = &mut j.tasks[q.task_id];
        t.result = result;
        t.status = if t.result["verification"] == "mismatch" {
            "error".into()
        } else {
            status.into()
        };
        guard.done = true;
        reply(200, json!(t))
    }
}
fn safe_error(f: &PluginFault) -> String {
    // 不透传宿主原始 message/details，防止账号凭据或上游错误正文泄漏。
    let kind = f
        .details
        .as_ref()
        .and_then(|v| v["kind"].as_str())
        .filter(|s| s.len() < 80 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
        .unwrap_or("");
    let msg = match f.code {
        ErrorCode::Unsupported => "模型或接口不支持",
        ErrorCode::InvalidInput => "模型参数无效或思考等级不支持",
        ErrorCode::PermissionDenied | ErrorCode::Rejected => "Key 权限、账号范围或访问策略拒绝",
        ErrorCode::Capacity => "限流、账号容量不足或无可用指定账号",
        ErrorCode::Timeout => "上游超时",
        ErrorCode::Cancelled => "执行已取消，可能已经消耗额度",
        ErrorCode::Upstream => "上游请求失败，请按请求 ID 查看 CPR 日志",
        _ => "宿主执行未完成，请核对 CPR 日志",
    };
    if kind.is_empty() {
        msg.into()
    } else {
        format!("{msg}（{kind}）")
    }
}
pub fn plugin() -> Result<ComposedPlugin, AuthorError> {
    let app = Arc::new(App::default());
    let routes = [
        ("GET", "api/snapshot"),
        ("GET", "api/presets"),
        ("POST", "api/presets"),
        ("POST", "api/models"),
        ("POST", "api/plan"),
        ("GET", "api/job"),
        ("POST", "api/control"),
        ("POST", "api/run"),
        ("POST", "api/run-batch"),
        ("POST", "api/task"),
        ("POST", "api/audit"),
    ]
    .into_iter()
    .map(|(m, p)| ManagementRoute {
        method: m.into(),
        path: p.into(),
        request_content_types: if m == "POST" {
            vec!["application/json".into()]
        } else {
            vec![]
        },
        response_content_types: vec!["application/json".into()],
    })
    .collect();
    let reg = ManagementRegistration {
        routes,
        resources: ["web/index.html", "web/app.js", "web/app.css"]
            .into_iter()
            .map(|p| ManagementResource {
                path: p.into(),
                public: false,
            })
            .collect(),
        pages: vec![ManagementPage {
            id: "account-dialogue".into(),
            title: "账号对话测试".into(),
            description: Some("多账号 × 多模型 × 多思考等级，并发真实对话测试".into()),
            entry: "web/index.html".into(),
            icon: None,
        }],
        callbacks: vec![],
    };
    PluginBuilder::from_json(include_bytes!("../../plugin.json"))?
        .management(reg, move |call| {
            let app = app.clone();
            async move {
                match app.handle(call).await {
                    Ok(r) => Ok(r),
                    Err(e) => reply(
                        if e.code == ErrorCode::InvalidInput {
                            400
                        } else if e.code == ErrorCode::Conflict {
                            409
                        } else {
                            502
                        },
                        json!({"error":{"code":e.code,"message":safe_error(&e)}}),
                    ),
                }
            }
        })?
        .build()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> Plan {
        Plan {
            key_id: "key".into(),
            accounts: vec!["a".into(), "b".into()],
            models: vec!["m1".into(), "m2".into()],
            efforts: vec!["low".into(), "high".into()],
            prompt: "test".into(),
            concurrency: 2,
            timeout_seconds: 90,
            selected_task_indexes: None,
        }
    }
    #[test]
    fn valid_plan() {
        assert!(plan().validate().is_ok());
        plugin().unwrap();
    }
    #[test]
    fn selected_task_indexes_validate() {
        let mut p = plan();
        p.selected_task_indexes = Some(vec![0, 7]);
        assert!(p.validate().is_ok());
        for indexes in [vec![], vec![0, 0], vec![8], vec![usize::MAX]] {
            p.selected_task_indexes = Some(indexes);
            assert!(p.validate().is_err());
        }
    }
    #[test]
    fn reject_duplicates() {
        let mut p = plan();
        p.accounts.push("a".into());
        assert!(p.validate().is_err())
    }
    #[test]
    fn limits() {
        let mut p = plan();
        p.concurrency = 0;
        assert!(p.validate().is_err());
        p = plan();
        p.models = (0..126).map(|n| n.to_string()).collect();
        assert!(p.validate().is_err());
        p = plan();
        p.efforts = vec!["imaginary".into()];
        assert!(p.validate().is_err())
    }
    #[test]
    fn concurrency_has_no_plugin_upper_limit_and_timeout_accepts_180() {
        let mut p = plan();
        p.timeout_seconds = 180;
        for c in [10, 100, 1000, usize::MAX] {
            p.concurrency = c;
            assert!(p.validate().is_ok());
        }
    }
    #[test]
    fn utf8_output_limit() {
        let mut s = "a".repeat(MAX_TEXT - 2);
        assert!(push_text(&mut s, "中文"));
        assert!(s.len() <= MAX_TEXT)
    }
    #[test]
    fn interruption_is_not_retriable() {
        let state = Arc::new(Mutex::new(State {
            next: 1,
            job: Some(Job {
                id: 1,
                plan: plan(),
                paused: false,
                tasks: vec![Task {
                    id: 0,
                    account_id: "a".into(),
                    account_name: "a".into(),
                    provider: "openai".into(),
                    model: "m".into(),
                    effort: "high".into(),
                    status: "running".into(),
                    result: json!({}),
                }],
            }),
        }));
        drop(RunningGuard {
            state: state.clone(),
            job: "1".into(),
            task: 0,
            done: false,
        });
        assert_eq!(
            state.lock().unwrap().job.as_ref().unwrap().tasks[0].status,
            "unknown"
        )
    }
    #[test]
    fn never_expose_secret_error() {
        let f = PluginFault::new(ErrorCode::Upstream, "Bearer SECRET");
        assert!(!safe_error(&f).contains("SECRET"))
    }
}
