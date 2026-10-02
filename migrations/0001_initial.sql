PRAGMA foreign_keys = ON;

CREATE TABLE customers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    phone TEXT,
    email TEXT,
    address TEXT,
    preferred_language TEXT CHECK (preferred_language IS NULL OR preferred_language IN ('es', 'en')),
    notes TEXT,
    normalized_phone TEXT,
    normalized_email TEXT,
    normalized_name TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE UNIQUE INDEX customers_normalized_phone_uq ON customers(normalized_phone) WHERE normalized_phone IS NOT NULL;
CREATE UNIQUE INDEX customers_normalized_email_uq ON customers(normalized_email) WHERE normalized_email IS NOT NULL;
CREATE INDEX customers_normalized_name_idx ON customers(normalized_name) WHERE normalized_name IS NOT NULL;

CREATE TABLE orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    customer_id INTEGER NOT NULL REFERENCES customers(id),
    status TEXT NOT NULL DEFAULT 'NEW' CHECK (status IN ('NEW','CONFIRMED','IN_PRODUCTION','READY','DELIVERED','CANCELLED')),
    original_text TEXT NOT NULL,
    normalized_description TEXT NOT NULL,
    requested_delivery_date TEXT,
    promised_delivery_date TEXT,
    created_by INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX orders_customer_id_idx ON orders(customer_id);
CREATE INDEX orders_status_idx ON orders(status);

CREATE TABLE order_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    product_type TEXT,
    description TEXT NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    length REAL,
    width REAL,
    height REAL,
    dimension_unit TEXT CHECK (dimension_unit IS NULL OR dimension_unit IN ('in','ft','cm','mm')),
    wood_species TEXT,
    finish TEXT,
    notes TEXT
);

CREATE TABLE work_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    description TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING',
    sequence INTEGER NOT NULL DEFAULT 0,
    assigned_to TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT
);

CREATE TABLE deliveries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL UNIQUE REFERENCES orders(id),
    delivered_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    delivered_by INTEGER NOT NULL,
    notes TEXT
);

CREATE TABLE attachments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    delivery_id INTEGER REFERENCES deliveries(id) ON DELETE CASCADE,
    type TEXT NOT NULL CHECK (type IN ('ORDER_REFERENCE','CUSTOMER_DRAWING','WORK_IN_PROGRESS','FINISHED_PRODUCT','DELIVERY_PROOF','OTHER')),
    path TEXT NOT NULL,
    telegram_file_id TEXT,
    telegram_message_id INTEGER,
    mime_type TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX attachments_order_id_idx ON attachments(order_id);

CREATE TABLE telegram_users (
    user_id INTEGER PRIMARY KEY,
    username TEXT,
    first_name TEXT,
    last_name TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE telegram_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    order_id INTEGER REFERENCES orders(id) ON DELETE CASCADE,
    delivery_id INTEGER REFERENCES deliveries(id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX telegram_sessions_user_chat_idx ON telegram_sessions(user_id, chat_id, expires_at);

CREATE TABLE telegram_updates (
    update_id INTEGER PRIMARY KEY,
    status TEXT NOT NULL,
    result_id INTEGER,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE ai_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    original_text TEXT NOT NULL,
    structured_result TEXT,
    model TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('completed','failed')),
    error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE order_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    description TEXT NOT NULL,
    created_by INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX order_events_order_id_idx ON order_events(order_id, created_at);
