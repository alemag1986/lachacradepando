use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{AppState, domain::Extraction};

pub const MAX_TEXT_CHARS: usize = 4000;
pub const MAX_IMAGES: usize = 4;
const MAX_OUTPUT_TOKENS: u32 = 800;

#[derive(Clone, Debug)]
pub struct InputImage {
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub telegram_file_id: Option<String>,
    pub telegram_message_id: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CompletionResponse {
    choices: Vec<Choice>,
}
#[derive(Debug, Serialize, Deserialize)]
struct Choice {
    message: AssistantMessage,
}
#[derive(Debug, Serialize, Deserialize)]
struct AssistantMessage {
    content: Value,
}

pub fn parse_extraction(content: &str) -> anyhow::Result<Extraction> {
    let trimmed = content.trim();
    let raw = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("``` ").or_else(|| s.strip_suffix("```")))
        .map(str::trim)
        .unwrap_or(trimmed);
    let value: Value = serde_json::from_str(raw).context("model response is not valid JSON")?;
    require_contract_keys(&value)?;
    let extraction: Extraction =
        serde_json::from_value(value).context("model response does not match extraction schema")?;
    extraction.validate().map_err(anyhow::Error::msg)?;
    Ok(extraction)
}

fn require_contract_keys(value: &Value) -> anyhow::Result<()> {
    fn require(value: &Value, keys: &[&str], location: &str) -> anyhow::Result<()> {
        let object = value
            .as_object()
            .with_context(|| format!("{location} must be an object"))?;
        for key in keys {
            anyhow::ensure!(
                object.contains_key(*key),
                "{location}.{key} is required; use null when unknown"
            );
        }
        Ok(())
    }
    require(
        value,
        &["customer", "order", "missing_fields"],
        "extraction",
    )?;
    require(
        &value["customer"],
        &[
            "name",
            "phone",
            "email",
            "address",
            "preferred_language",
            "notes",
        ],
        "customer",
    )?;
    let order = &value["order"];
    require(
        order,
        &["description", "requested_delivery_date", "items"],
        "order",
    )?;
    let items = order["items"]
        .as_array()
        .context("order.items must be an array")?;
    for (index, item) in items.iter().enumerate() {
        require(
            item,
            &[
                "product_type",
                "description",
                "quantity",
                "length",
                "width",
                "height",
                "dimension_unit",
                "wood_species",
                "finish",
                "notes",
            ],
            &format!("order.items[{index}]"),
        )?;
    }
    Ok(())
}

pub async fn extract(
    state: &AppState,
    text: &str,
    images: &[InputImage],
) -> anyhow::Result<(Extraction, String)> {
    extract_at(
        state,
        text,
        images,
        "https://openrouter.ai/api/v1/chat/completions",
    )
    .await
}

pub async fn extract_at(
    state: &AppState,
    text: &str,
    images: &[InputImage],
    endpoint: &str,
) -> anyhow::Result<(Extraction, String)> {
    if text.chars().count() > MAX_TEXT_CHARS {
        bail!("pedido text exceeds 4000 characters");
    }
    if images.len() > MAX_IMAGES {
        bail!("pedido has more than four images");
    }
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let prompt = format!(
        "Extract only information present in the message and images. Translate meaning but preserve customer intent. Normalize quantities and measurements. Never invent missing information. Return null when unknown. Do not calculate price, select customer IDs, or create production tasks. Today's date is {today}; resolve relative dates only when unambiguous. Return JSON only using exactly this contract: {{\"customer\":{{\"name\":string|null,\"phone\":string|null,\"email\":string|null,\"address\":string|null,\"preferred_language\":\"es\"|\"en\"|null,\"notes\":string|null}},\"order\":{{\"description\":string,\"requested_delivery_date\":\"YYYY-MM-DD\"|null,\"items\":[{{\"product_type\":string|null,\"description\":string,\"quantity\":positive integer,\"length\":number|null,\"width\":number|null,\"height\":number|null,\"dimension_unit\":\"in\"|\"ft\"|\"cm\"|\"mm\"|null,\"wood_species\":string|null,\"finish\":string|null,\"notes\":string|null}}]}},\"missing_fields\":[string]}}. Include every key and use null for unknown values."
    );
    let schema = json!({"type":"object","additionalProperties":false,"required":["customer","order","missing_fields"],"properties":{
        "customer":{"type":"object","additionalProperties":false,"required":["name","phone","email","address","preferred_language","notes"],"properties":{"name":{"type":["string","null"]},"phone":{"type":["string","null"]},"email":{"type":["string","null"]},"address":{"type":["string","null"]},"preferred_language":{"enum":["es","en",null]},"notes":{"type":["string","null"]}}},
        "order":{"type":"object","additionalProperties":false,"required":["description","requested_delivery_date","items"],"properties":{"description":{"type":"string"},"requested_delivery_date":{"type":["string","null"]},"items":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["product_type","description","quantity","length","width","height","dimension_unit","wood_species","finish","notes"],"properties":{"product_type":{"type":["string","null"]},"description":{"type":"string"},"quantity":{"type":"integer","minimum":1},"length":{"type":["number","null"]},"width":{"type":["number","null"]},"height":{"type":["number","null"]},"dimension_unit":{"enum":["in","ft","cm","mm",null]},"wood_species":{"type":["string","null"]},"finish":{"type":["string","null"]},"notes":{"type":["string","null"]}}}}}},"missing_fields":{"type":"array","items":{"type":"string"}}}});
    let mut messages =
        vec![json!({"role":"system","content":format!("{prompt}\nSchema: {schema}")})];
    let mut content = vec![json!({"type":"text","text":text})];
    for image in images {
        content.push(json!({"type":"image_url","image_url":{"url":format!("data:{};base64,{}",image.mime_type,STANDARD.encode(&image.bytes))}}));
    }
    messages.push(json!({"role":"user","content":content}));
    let models = [
        state.config.openrouter_model.as_str(),
        state.config.openrouter_fallback_model.as_str(),
    ];
    let mut last_error = String::from("extraction failed");
    for (attempt, model) in models.into_iter().enumerate() {
        let mut request_messages = messages.clone();
        if attempt == 1 {
            request_messages.push(json!({"role":"user","content":format!("The previous model response failed validation: {last_error}. Correct it and return only valid JSON.")}));
        }
        let request = json!({"model":model,"messages":request_messages,"max_tokens":MAX_OUTPUT_TOKENS,"temperature":0});
        let result = async {
            let response = state
                .http
                .post(endpoint)
                .bearer_auth(&state.config.openrouter_api_key)
                .json(&request)
                .send()
                .await?
                .error_for_status()?;
            let body: CompletionResponse = response.json().await?;
            let raw = body
                .choices
                .first()
                .context("OpenRouter returned no choices")?
                .message
                .content
                .as_str()
                .context("OpenRouter content was not text")?
                .to_owned();
            parse_extraction(&raw).map(|extraction| (extraction, model.to_owned()))
        }
        .await;
        match result {
            Ok(parsed) => return Ok(parsed),
            Err(error) => last_error = format!("{error:#}"),
        }
    }
    bail!("{last_error}")
}
