ALTER TABLE telegram_updates ADD COLUMN extraction_json TEXT;
ALTER TABLE telegram_updates ADD COLUMN extraction_model TEXT;

CREATE TABLE telegram_album_members (
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    media_group_id TEXT NOT NULL,
    message_id INTEGER NOT NULL,
    update_id INTEGER NOT NULL,
    file_id TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    command_text TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','attached')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY(user_id, chat_id, media_group_id, message_id),
    UNIQUE(update_id)
);
CREATE INDEX telegram_album_members_lookup_idx
    ON telegram_album_members(user_id, chat_id, media_group_id, created_at);

CREATE UNIQUE INDEX attachments_telegram_identity_uq
    ON attachments(order_id, COALESCE(delivery_id, 0), type, telegram_file_id, telegram_message_id)
    WHERE telegram_file_id IS NOT NULL AND telegram_message_id IS NOT NULL;
