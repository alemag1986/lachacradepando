use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

use anyhow::Context;
use axum::{
    Router,
    body::Bytes,
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use image::{ImageFormat, ImageReader, imageops::FilterType};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::{
    AppState,
    ai::{self, InputImage},
};

const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_PIXELS: u64 = 40_000_000;
type ChatMutex = tokio::sync::Mutex<()>;
type ChatLockMap = HashMap<(i64, i64), Weak<ChatMutex>>;
static CHAT_LOCKS: OnceLock<tokio::sync::Mutex<ChatLockMap>> = OnceLock::new();

pub fn router() -> Router<crate::AppState> {
    router_with_api_endpoints(ApiEndpoints::production())
}

#[derive(Clone)]
pub struct ApiEndpoints {
    telegram_api: String,
    telegram_file: String,
    openrouter: String,
}
impl ApiEndpoints {
    fn production() -> Self {
        Self {
            telegram_api: "https://api.telegram.org".into(),
            telegram_file: "https://api.telegram.org/file".into(),
            openrouter: "https://openrouter.ai/api/v1/chat/completions".into(),
        }
    }
    #[doc(hidden)]
    pub fn for_test(telegram_api: String, telegram_file: String, openrouter: String) -> Self {
        Self {
            telegram_api,
            telegram_file,
            openrouter,
        }
    }
}
#[doc(hidden)]
pub fn router_with_api_endpoints(endpoints: ApiEndpoints) -> Router<crate::AppState> {
    Router::new()
        .route("/telegram/webhook", post(webhook))
        .layer(Extension(endpoints))
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Message>,
}
#[derive(Debug, Deserialize)]
struct Message {
    message_id: i64,
    from: Option<User>,
    chat: Chat,
    text: Option<String>,
    photo: Option<Vec<PhotoSize>>,
    caption: Option<String>,
    #[serde(default)]
    media_group_id: Option<String>,
}
#[derive(Debug, Deserialize)]
struct User {
    id: i64,
}
#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Clone, Debug, Deserialize)]
struct PhotoSize {
    file_id: String,
    width: u32,
    height: u32,
}
#[derive(Clone)]
struct PhotoRef {
    photo: PhotoSize,
    message_id: i64,
}
#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
}
#[derive(Debug, Deserialize)]
struct TelegramFile {
    file_path: String,
    file_size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Pedido(String),
    Entrega(i64),
    Other,
}

pub fn parse_command(text: &str) -> Command {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();
    if lower == "pedido" || lower.starts_with("pedido\n") || lower.starts_with("pedido\r\n") {
        Command::Pedido(trimmed.get(6..).unwrap_or_default().trim().to_owned())
    } else {
        let mut parts = trimmed.split_whitespace();
        match (
            parts.next().map(str::to_lowercase).as_deref(),
            parts.next(),
            parts.next(),
        ) {
            (Some("entrega"), Some(id), None) => id
                .parse::<i64>()
                .ok()
                .filter(|n| *n > 0)
                .map(Command::Entrega)
                .unwrap_or(Command::Other),
            _ => Command::Other,
        }
    }
}

pub fn validate_photo(bytes: &[u8]) -> anyhow::Result<(String, u32, u32)> {
    let (mime, decoded) = decode_photo(bytes)?;
    let (width, height) = (decoded.width(), decoded.height());
    Ok((mime, width, height))
}

fn decode_photo(bytes: &[u8]) -> anyhow::Result<(String, image::DynamicImage)> {
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_IMAGE_BYTES,
        "image size is invalid"
    );
    let format = image::guess_format(bytes)?;
    let mime = match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png => "image/png",
        ImageFormat::WebP => "image/webp",
        _ => anyhow::bail!("unsupported image format"),
    };
    let mut reader = ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode()?;
    anyhow::ensure!(
        decoded.width() as u64 * decoded.height() as u64 <= MAX_PIXELS,
        "image dimensions exceed limit"
    );
    Ok((mime.into(), decoded))
}

fn safe_telegram_path(path: &str) -> anyhow::Result<()> {
    let p = Path::new(path);
    anyhow::ensure!(
        !p.is_absolute() && p.components().all(|c| matches!(c, Component::Normal(_))),
        "invalid Telegram file path"
    );
    anyhow::ensure!(
        !path.contains('\\') && !path.contains('\0'),
        "invalid Telegram file path"
    );
    Ok(())
}

pub fn validate_telegram_file_path(path: &str) -> anyhow::Result<()> {
    safe_telegram_path(path)
}

async fn webhook(
    State(state): State<AppState>,
    Extension(endpoints): Extension<ApiEndpoints>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let expected = state.config.telegram_webhook_secret.as_bytes();
    let supplied = headers
        .get("x-telegram-bot-api-secret-token")
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    if expected.is_empty()
        || expected.len() != supplied.len()
        || !bool::from(expected.ct_eq(supplied))
    {
        return StatusCode::UNAUTHORIZED;
    }
    if body.len() > 1024 * 1024 {
        return StatusCode::PAYLOAD_TOO_LARGE;
    }
    let update: Update = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    // Authenticate before persisting anything about the update.
    let Some(message) = update.message.as_ref() else {
        return StatusCode::OK;
    };
    let user_id = message
        .from
        .as_ref()
        .map(|user| user.id)
        .unwrap_or_default();
    if !is_authorized(&state, message) {
        return StatusCode::OK;
    }
    if persist_album_member(&state, update.update_id, message)
        .await
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    if message.media_group_id.is_some() {
        let content = message
            .text
            .as_deref()
            .or(message.caption.as_deref())
            .unwrap_or("");
        let pause = if matches!(parse_command(content), Command::Pedido(_)) {
            1200
        } else {
            1500
        };
        tokio::time::sleep(Duration::from_millis(pause)).await;
    }
    let _guard = chat_lock(user_id, message.chat.id).await;
    match record_update(&state, update.update_id).await {
        Ok(true) => {
            let id = update.update_id;
            match handle_update(&state, &endpoints, update).await {
                Ok(()) => StatusCode::OK,
                Err(error) => {
                    let _ = error;
                    tracing::error!("telegram update processing failed");
                    let _ = finish_update(&state, id, "failed").await;
                    StatusCode::SERVICE_UNAVAILABLE
                }
            }
        }
        Ok(false) => StatusCode::OK,
        Err(_) => {
            tracing::error!("telegram update persistence failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

async fn persist_album_member(
    state: &AppState,
    update_id: i64,
    message: &Message,
) -> anyhow::Result<()> {
    let (Some(group), Some(variants)) = (&message.media_group_id, &message.photo) else {
        return Ok(());
    };
    let Some(photo) = variants.iter().max_by_key(|p| (p.width, p.height)) else {
        return Ok(());
    };
    let text = message
        .caption
        .as_deref()
        .or(message.text.as_deref())
        .unwrap_or("");
    let command_text = matches!(parse_command(text), Command::Pedido(_)).then_some(text);
    sqlx::query("DELETE FROM telegram_album_members WHERE created_at < strftime('%Y-%m-%dT%H:%M:%fZ','now','-7 days')").execute(&state.pool).await?;
    sqlx::query("INSERT INTO telegram_album_members(user_id,chat_id,media_group_id,message_id,update_id,file_id,width,height,command_text) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(user_id,chat_id,media_group_id,message_id) DO UPDATE SET command_text=COALESCE(telegram_album_members.command_text,excluded.command_text)")
        .bind(message.from.as_ref().map(|u|u.id).unwrap_or_default()).bind(message.chat.id).bind(group).bind(message.message_id).bind(update_id).bind(&photo.file_id).bind(photo.width).bind(photo.height).bind(command_text).execute(&state.pool).await?;
    Ok(())
}

async fn chat_lock(user_id: i64, chat_id: i64) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = CHAT_LOCKS
            .get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
            .lock()
            .await;
        locks.retain(|_, weak| weak.strong_count() > 0);
        let key = (user_id, chat_id);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key, Arc::downgrade(&lock));
            lock
        }
    };
    lock.lock_owned().await
}

async fn record_update(state: &AppState, update_id: i64) -> anyhow::Result<bool> {
    sqlx::query("INSERT INTO telegram_updates(update_id,status) VALUES(?, 'processing') ON CONFLICT(update_id) DO NOTHING")
        .bind(update_id).execute(&state.pool).await?;
    let status: String =
        sqlx::query_scalar("SELECT status FROM telegram_updates WHERE update_id=?")
            .bind(update_id)
            .fetch_one(&state.pool)
            .await?;
    Ok(status != "completed")
}

async fn handle_update(
    state: &AppState,
    endpoints: &ApiEndpoints,
    update: Update,
) -> anyhow::Result<()> {
    let Some(message) = update.message else {
        finish_update(state, update.update_id, "ignored").await?;
        return Ok(());
    };
    let user_id = message.from.as_ref().map(|u| u.id).unwrap_or_default();
    if !is_authorized(state, &message) {
        return Ok(());
    }
    let text = message
        .text
        .as_deref()
        .or(message.caption.as_deref())
        .unwrap_or("");
    let original_text = text;
    let command = parse_command(text);
    let photos = if let Some(group) = &message.media_group_id {
        let entries:Vec<(String,i64)> = sqlx::query_as("SELECT file_id,message_id FROM telegram_album_members WHERE user_id=? AND chat_id=? AND media_group_id=? ORDER BY created_at,message_id")
            .bind(user_id).bind(message.chat.id).bind(group).fetch_all(&state.pool).await?;
        let pedido:Option<String> = sqlx::query_scalar("SELECT command_text FROM telegram_album_members WHERE user_id=? AND chat_id=? AND media_group_id=? AND command_text IS NOT NULL LIMIT 1")
            .bind(user_id).bind(message.chat.id).bind(group).fetch_optional(&state.pool).await?;
        if entries.len() > ai::MAX_IMAGES {
            send_message(
                state,
                endpoints,
                message.chat.id,
                "Los álbumes admiten un máximo de cuatro fotos.",
            )
            .await?;
            finish_update(state, update.update_id, "completed").await?;
            return Ok(());
        }
        if !matches!(command, Command::Pedido(_)) && pedido.is_none() {
            attach_delivery_album(
                state,
                endpoints,
                &message,
                user_id,
                &entries
                    .iter()
                    .map(|(file_id, message_id)| PhotoRef {
                        photo: PhotoSize {
                            file_id: file_id.clone(),
                            width: 0,
                            height: 0,
                        },
                        message_id: *message_id,
                    })
                    .collect::<Vec<_>>(),
            )
            .await?;
            sqlx::query("UPDATE telegram_album_members SET status='attached' WHERE user_id=? AND chat_id=? AND media_group_id=?").bind(user_id).bind(message.chat.id).bind(group).execute(&state.pool).await?;
            finish_update(state, update.update_id, "completed").await?;
            return Ok(());
        }
        if !matches!(command, Command::Pedido(_)) && pedido.is_some() {
            let committed_order:Option<i64> = sqlx::query_scalar("SELECT result_id FROM telegram_updates WHERE update_id=(SELECT update_id FROM telegram_album_members WHERE user_id=? AND chat_id=? AND media_group_id=? AND command_text IS NOT NULL LIMIT 1) AND status IN ('committed','completed')")
                .bind(user_id).bind(message.chat.id).bind(group).fetch_optional(&state.pool).await?.flatten();
            if let Some(order_id) = committed_order {
                attach_reference_album(state, endpoints, &message, user_id, group, order_id)
                    .await?;
            }
            finish_update(state, update.update_id, "completed").await?;
            return Ok(());
        }
        entries
            .into_iter()
            .map(|(file_id, message_id)| PhotoRef {
                photo: PhotoSize {
                    file_id,
                    width: 0,
                    height: 0,
                },
                message_id,
            })
            .collect()
    } else {
        message
            .photo
            .as_deref()
            .and_then(|v| v.iter().max_by_key(|p| (p.width, p.height)))
            .map(|p| {
                vec![PhotoRef {
                    photo: p.clone(),
                    message_id: message.message_id,
                }]
            })
            .unwrap_or_default()
    };
    match command {
        Command::Pedido(inline) => {
            process_pedido(
                state,
                endpoints,
                PedidoInput {
                    update_id: update.update_id,
                    message: &message,
                    user_id,
                    text: if inline.is_empty() { text } else { &inline },
                    photos: &photos,
                    original_text,
                },
            )
            .await?
        }
        Command::Entrega(order_id) => {
            process_delivery(
                state,
                endpoints,
                update.update_id,
                &message,
                user_id,
                order_id,
            )
            .await?
        }
        Command::Other => {
            if message.media_group_id.is_none()
                && !attach_delivery_photos(state, endpoints, &message, user_id).await?
            {
                send_message(state, endpoints, message.chat.id, "Envía pedido para registrar un pedido o entrega ID para registrar una entrega.").await?;
            }
        }
    }
    finish_update(state, update.update_id, "completed").await?;
    Ok(())
}

fn is_authorized(state: &AppState, message: &Message) -> bool {
    let user_id = message.from.as_ref().map(|u| u.id).unwrap_or_default();
    state.config.telegram_allowed_user_ids.contains(&user_id)
        && (if state.config.telegram_allowed_chat_ids.is_empty() {
            message.chat.kind == "private"
        } else {
            state
                .config
                .telegram_allowed_chat_ids
                .contains(&message.chat.id)
        })
}

async fn finish_update(state: &AppState, id: i64, status: &str) -> anyhow::Result<()> {
    if status == "failed" {
        sqlx::query(
            "UPDATE telegram_updates SET status='failed' WHERE update_id=? AND status='processing'",
        )
        .bind(id)
        .execute(&state.pool)
        .await?;
    } else {
        sqlx::query("UPDATE telegram_updates SET status=? WHERE update_id=?")
            .bind(status)
            .bind(id)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

struct PedidoInput<'a> {
    update_id: i64,
    message: &'a Message,
    user_id: i64,
    text: &'a str,
    photos: &'a [PhotoRef],
    original_text: &'a str,
}

async fn process_pedido(
    state: &AppState,
    endpoints: &ApiEndpoints,
    input: PedidoInput<'_>,
) -> anyhow::Result<()> {
    let PedidoInput {
        update_id,
        message,
        user_id,
        text,
        photos,
        original_text,
    } = input;
    if original_text.chars().count() > ai::MAX_TEXT_CHARS {
        send_message(
            state,
            endpoints,
            message.chat.id,
            "El pedido supera el límite de 4000 caracteres.",
        )
        .await?;
        return Ok(());
    }
    if photos.len() > ai::MAX_IMAGES {
        send_message(
            state,
            endpoints,
            message.chat.id,
            "Envía hasta cuatro fotos de referencia.",
        )
        .await?;
        return Ok(());
    }
    let mut imgs = Vec::new();
    for photo in photos {
        match download_photo(state, endpoints, &photo.photo.file_id).await {
            Ok(mut image) => {
                image.telegram_message_id = Some(photo.message_id);
                imgs.push(image)
            }
            Err(_) => {
                tracing::warn!("Telegram photo download or validation failed");
                send_message(
                    state,
                    endpoints,
                    message.chat.id,
                    "No pude validar una foto. Reenviaré el pedido al recibir el reintento.",
                )
                .await?;
                anyhow::bail!("photo download or validation failed")
            }
        }
    }
    let committed = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT result_id FROM telegram_updates WHERE update_id=? AND status='committed'",
    )
    .bind(update_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    let (order_id, summary) = if let Some(order_id) = committed {
        (order_id, format!("Pedido #{order_id} creado"))
    } else {
        let staged:Option<(String,String)> = sqlx::query_as("SELECT extraction_json,extraction_model FROM telegram_updates WHERE update_id=? AND extraction_json IS NOT NULL AND extraction_model IS NOT NULL")
            .bind(update_id).fetch_optional(&state.pool).await?;
        let (extraction, model) = if let Some((json, model)) = staged {
            (ai::parse_extraction(&json)?, model)
        } else {
            let result = ai::extract_at(state, text, &imgs, &endpoints.openrouter).await;
            let (extraction, model) = match result {
                Ok(value) => value,
                Err(error) => {
                    let model = state.config.openrouter_fallback_model.clone();
                    let _=sqlx::query("INSERT INTO ai_requests(original_text,structured_result,model,status,error) VALUES(?,NULL,?,'failed',?)").bind(original_text).bind(model).bind(format!("{error:#}")).execute(&state.pool).await;
                    send_message(state,endpoints,message.chat.id,"No pude entender el pedido con seguridad. Inténtalo de nuevo con más detalle.").await?;
                    return Ok(());
                }
            };
            let encoded = serde_json::to_string(&extraction)?;
            sqlx::query("UPDATE telegram_updates SET extraction_json=?,extraction_model=? WHERE update_id=? AND status='processing'").bind(encoded).bind(&model).bind(update_id).execute(&state.pool).await?;
            (extraction, model)
        };
        let id = crate::db::create_order_for_update(
            &state.pool,
            &extraction,
            original_text,
            user_id,
            &model,
            update_id,
        )
        .await?;
        let summary = format!(
            "Pedido #{id} creado\nCliente: {}\nProducto: {}\nMaterial: {}\nMedidas: {}\nEntrega: {}",
            extraction.customer.name.as_deref().unwrap_or("Sin nombre"),
            extraction
                .order
                .items
                .first()
                .and_then(|i| i.product_type.as_deref())
                .unwrap_or("Por definir"),
            extraction
                .order
                .items
                .first()
                .and_then(|i| i.wood_species.as_deref())
                .unwrap_or("Por definir"),
            extraction
                .order
                .items
                .first()
                .map(format_dimensions)
                .unwrap_or_else(|| "Por definir".into()),
            extraction
                .order
                .requested_delivery_date
                .as_deref()
                .unwrap_or("Sin fecha")
        );
        (id, summary)
    };
    if let Some(group) = &message.media_group_id {
        attach_reference_album(state, endpoints, message, user_id, group, order_id).await?;
    } else if !imgs.is_empty() {
        store_order_photos(state, order_id, &imgs, message.message_id).await?;
    }
    send_message(state, endpoints, message.chat.id, &summary).await?;
    finish_update(state, update_id, "completed").await?;
    Ok(())
}

fn format_dimensions(item: &crate::domain::OrderItemInput) -> String {
    let dims = [item.length, item.width, item.height]
        .into_iter()
        .flatten()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("x");
    if dims.is_empty() {
        "Por definir".into()
    } else {
        format!("{dims} {}", item.dimension_unit.as_deref().unwrap_or(""))
            .trim()
            .to_owned()
    }
}

async fn process_delivery(
    state: &AppState,
    endpoints: &ApiEndpoints,
    update_id: i64,
    message: &Message,
    user_id: i64,
    order_id: i64,
) -> anyhow::Result<()> {
    let _delivery_id = match crate::db::deliver_order_for_update(
        &state.pool,
        order_id,
        user_id,
        message.chat.id,
        None,
        update_id,
    )
    .await
    {
        Ok(id) => id,
        Err(_) => {
            send_message(
                state,
                endpoints,
                message.chat.id,
                "No encontré ese pedido o no se pudo registrar la entrega.",
            )
            .await?;
            return Ok(());
        }
    };
    send_message(state, endpoints, message.chat.id, &format!("Pedido #{order_id} marcado como entregado. Las fotos enviadas durante los próximos 15 minutos se adjuntarán a esta entrega.")).await?;
    finish_update(state, update_id, "completed").await?;
    Ok(())
}

async fn attach_delivery_photos(
    state: &AppState,
    endpoints: &ApiEndpoints,
    message: &Message,
    user_id: i64,
) -> anyhow::Result<bool> {
    let Some(photo) = message
        .photo
        .as_ref()
        .and_then(|p| p.iter().max_by_key(|v| (v.width, v.height)))
    else {
        return Ok(false);
    };
    let session = sqlx::query_as::<_, (i64, i64)>("SELECT order_id,delivery_id FROM telegram_sessions WHERE user_id=? AND chat_id=? AND julianday(expires_at)>julianday(strftime('%Y-%m-%dT%H:%M:%fZ','now')) ORDER BY julianday(expires_at) DESC LIMIT 1")
        .bind(user_id).bind(message.chat.id).fetch_optional(&state.pool).await?;
    let Some((order_id, delivery_id)) = session else {
        return Ok(false);
    };
    let image = download_photo(state, endpoints, &photo.file_id).await?;
    if delivery_photo_exists(
        state,
        order_id,
        delivery_id,
        &photo.file_id,
        message.message_id,
    )
    .await?
    {
        return Ok(true);
    }
    let (path, mime) = persist_photo(state, order_id, &image).await?;
    crate::db::attach_photo_with_metadata(
        &state.pool,
        order_id,
        Some(delivery_id),
        crate::db::PhotoMetadata {
            kind: "DELIVERY_PROOF",
            path: &path,
            file_id: &photo.file_id,
            message_id: message.message_id,
            mime: &mime,
        },
    )
    .await?;
    send_message(
        state,
        endpoints,
        message.chat.id,
        "Foto de entrega guardada.",
    )
    .await?;
    Ok(true)
}

async fn attach_delivery_album(
    state: &AppState,
    endpoints: &ApiEndpoints,
    message: &Message,
    user_id: i64,
    photos: &[PhotoRef],
) -> anyhow::Result<()> {
    let session = sqlx::query_as::<_, (i64, i64)>("SELECT order_id,delivery_id FROM telegram_sessions WHERE user_id=? AND chat_id=? AND julianday(expires_at)>julianday(strftime('%Y-%m-%dT%H:%M:%fZ','now')) ORDER BY julianday(expires_at) DESC LIMIT 1")
        .bind(user_id).bind(message.chat.id).fetch_optional(&state.pool).await?;
    let Some((order_id, delivery_id)) = session else {
        return Ok(());
    };
    for reference in photos {
        if delivery_photo_exists(
            state,
            order_id,
            delivery_id,
            &reference.photo.file_id,
            reference.message_id,
        )
        .await?
        {
            continue;
        }
        let photo = download_photo(state, endpoints, &reference.photo.file_id).await?;
        let (path, mime) = persist_photo(state, order_id, &photo).await?;
        crate::db::attach_photo_with_metadata(
            &state.pool,
            order_id,
            Some(delivery_id),
            crate::db::PhotoMetadata {
                kind: "DELIVERY_PROOF",
                path: &path,
                file_id: &reference.photo.file_id,
                message_id: reference.message_id,
                mime: &mime,
            },
        )
        .await?;
    }
    send_message(
        state,
        endpoints,
        message.chat.id,
        "Fotos de entrega guardadas.",
    )
    .await?;
    Ok(())
}

async fn attach_reference_album(
    state: &AppState,
    endpoints: &ApiEndpoints,
    message: &Message,
    user_id: i64,
    group: &str,
    order_id: i64,
) -> anyhow::Result<()> {
    let entries:Vec<(String,i64)>=sqlx::query_as("SELECT file_id,message_id FROM telegram_album_members WHERE user_id=? AND chat_id=? AND media_group_id=? ORDER BY created_at,message_id LIMIT 4")
        .bind(user_id).bind(message.chat.id).bind(group).fetch_all(&state.pool).await?;
    let total:i64=sqlx::query_scalar("SELECT count(*) FROM telegram_album_members WHERE user_id=? AND chat_id=? AND media_group_id=?")
        .bind(user_id).bind(message.chat.id).bind(group).fetch_one(&state.pool).await?;
    if total > ai::MAX_IMAGES as i64 {
        send_message(
            state,
            endpoints,
            message.chat.id,
            "Los álbumes admiten un máximo de cuatro fotos; se guardaron las primeras cuatro.",
        )
        .await?;
    }
    for (file_id, message_id) in entries {
        let previous:Option<i64>=sqlx::query_scalar("SELECT id FROM attachments WHERE type='DELIVERY_PROOF' AND telegram_file_id=? AND telegram_message_id=? LIMIT 1")
            .bind(&file_id).bind(message_id).fetch_optional(&state.pool).await?;
        if let Some(attachment_id) = previous {
            sqlx::query("UPDATE attachments SET order_id=?,delivery_id=NULL,type='ORDER_REFERENCE' WHERE id=?")
                .bind(order_id).bind(attachment_id).execute(&state.pool).await?;
            continue;
        }
        let exists:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments WHERE order_id=? AND type='ORDER_REFERENCE' AND telegram_file_id=? AND telegram_message_id=?)")
            .bind(order_id).bind(&file_id).bind(message_id).fetch_one(&state.pool).await?;
        if exists != 0 {
            continue;
        }
        let image = download_photo(state, endpoints, &file_id).await?;
        let (path, mime) = persist_photo(state, order_id, &image).await?;
        crate::db::attach_photo_with_metadata(
            &state.pool,
            order_id,
            None,
            crate::db::PhotoMetadata {
                kind: "ORDER_REFERENCE",
                path: &path,
                file_id: &file_id,
                message_id,
                mime: &mime,
            },
        )
        .await?;
    }
    sqlx::query("UPDATE telegram_album_members SET status='attached' WHERE user_id=? AND chat_id=? AND media_group_id=?")
        .bind(user_id).bind(message.chat.id).bind(group).execute(&state.pool).await?;
    Ok(())
}

async fn delivery_photo_exists(
    state: &AppState,
    order_id: i64,
    delivery_id: i64,
    file_id: &str,
    message_id: i64,
) -> anyhow::Result<bool> {
    let exists: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments WHERE order_id=? AND delivery_id=? AND type='DELIVERY_PROOF' AND telegram_file_id=? AND telegram_message_id=?)")
        .bind(order_id).bind(delivery_id).bind(file_id).bind(message_id).fetch_one(&state.pool).await?;
    Ok(exists != 0)
}

async fn download_photo(
    state: &AppState,
    endpoints: &ApiEndpoints,
    file_id: &str,
) -> anyhow::Result<InputImage> {
    let api = format!(
        "{}/bot{}/getFile",
        endpoints.telegram_api, state.config.telegram_bot_token
    );
    let response: TelegramResponse<TelegramFile> = state
        .http
        .get(api)
        .query(&[("file_id", file_id)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    anyhow::ensure!(response.ok, "Telegram getFile failed");
    let file = response.result.context("Telegram omitted file metadata")?;
    anyhow::ensure!(
        file.file_size
            .is_none_or(|size| size <= MAX_IMAGE_BYTES as u64),
        "photo exceeds size limit"
    );
    let path = file.file_path;
    safe_telegram_path(&path)?;
    let url = format!(
        "{}/bot{}/{}",
        endpoints.telegram_file, state.config.telegram_bot_token, path
    );
    let response = state.http.get(url).send().await?.error_for_status()?;
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_IMAGE_BYTES as u64),
        "photo exceeds size limit"
    );
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_IMAGE_BYTES,
            "photo exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let (mime, decoded) = decode_photo(&bytes)?;
    let (width, height) = (decoded.width(), decoded.height());
    let longest = width.max(height);
    let resized = if longest > 1600 {
        decoded.resize(1600, 1600, FilterType::Lanczos3)
    } else {
        decoded
    };
    let mut out = std::io::Cursor::new(Vec::new());
    resized.write_to(&mut out, ImageFormat::Jpeg)?;
    let _ = mime;
    Ok(InputImage {
        mime_type: "image/jpeg".into(),
        bytes: out.into_inner(),
        telegram_file_id: Some(file_id.to_owned()),
        telegram_message_id: None,
    })
}

async fn store_order_photos(
    state: &AppState,
    order_id: i64,
    images: &[InputImage],
    message_id: i64,
) -> anyhow::Result<()> {
    for img in images {
        let file_id = img.telegram_file_id.as_deref().unwrap_or("");
        let message = img.telegram_message_id.unwrap_or(message_id);
        let exists: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments WHERE order_id=? AND type='ORDER_REFERENCE' AND telegram_file_id=? AND telegram_message_id=?)")
            .bind(order_id).bind(file_id).bind(message).fetch_one(&state.pool).await?;
        if exists != 0 {
            continue;
        }
        let (path, mime) = persist_photo(state, order_id, img).await?;
        crate::db::attach_photo_with_metadata(
            &state.pool,
            order_id,
            None,
            crate::db::PhotoMetadata {
                kind: "ORDER_REFERENCE",
                path: &path,
                file_id,
                message_id: message,
                mime: &mime,
            },
        )
        .await?;
    }
    Ok(())
}

async fn persist_photo(
    state: &AppState,
    order_id: i64,
    image: &InputImage,
) -> anyhow::Result<(String, String)> {
    anyhow::ensure!(order_id > 0, "invalid order ID");
    let root = PathBuf::from(&state.config.data_dir)
        .join("photos")
        .join(order_id.to_string());
    tokio::fs::create_dir_all(&root).await?;
    let name = format!("{}.jpg", Uuid::new_v4());
    let path = root.join(&name);
    tokio::fs::write(&path, &image.bytes).await?;
    Ok((format!("{order_id}/{name}"), "image/jpeg".into()))
}

async fn send_message(
    state: &AppState,
    endpoints: &ApiEndpoints,
    chat_id: i64,
    text: &str,
) -> anyhow::Result<()> {
    let url = format!(
        "{}/bot{}/sendMessage",
        endpoints.telegram_api, state.config.telegram_bot_token
    );
    let response: TelegramResponse<Value> = state
        .http
        .post(url)
        .json(&json!({"chat_id":chat_id,"text":text}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    anyhow::ensure!(response.ok, "Telegram sendMessage failed");
    Ok(())
}
